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

pub async fn sync_once(
    store: super::store::CanvasTrackerStore,
    settings: &super::types::CanvasTrackerSettings,
    token: &str,
    now: DateTime<Utc>,
) -> Result<super::types::SyncSummary, String> {
    let client = super::client::CanvasClient::new(&settings.host, token.to_string())
        .map_err(|e| e.to_string())?;
    let courses: Vec<CanvasCourseDto> = client
        .get_json(super::client::CanvasEndpoint::Courses)
        .await
        .map_err(|e| e.to_string())?;

    let mut used_courses = Vec::new();
    let mut ignored = 0usize;
    for course in courses.iter() {
        let id = id_to_string(&course.id);
        let name = course.name.clone().unwrap_or_default();
        if super::policy::course_matches_allowlist(Some(&id), &name, &settings.allowlisted_courses)
        {
            used_courses.push((id, name));
        } else {
            ignored += 1;
        }
    }

    let mut tasks = Vec::new();
    for (course_id, course_name) in used_courses.iter() {
        let assignments: Vec<CanvasAssignmentDto> = client
            .get_json(super::client::CanvasEndpoint::Assignments {
                course_id: course_id.clone(),
            })
            .await
            .map_err(|e| e.to_string())?;
        for assignment in assignments {
            tasks.push(normalize_assignment(
                course_id,
                course_name,
                assignment,
                now,
            ));
        }
    }

    let upserted = store.upsert_tasks(&tasks).map_err(|e| e.to_string())?;
    let summary = super::types::SyncSummary {
        synced: true,
        courses_seen: courses.len(),
        courses_used: used_courses.len(),
        courses_ignored: ignored,
        assignments_seen: tasks.len(),
        tasks_upserted: upserted,
        previous_tasks_preserved: true,
        errors: vec![],
        synced_at: now.to_rfc3339(),
    };
    store.record_sync_run(&summary).map_err(|e| e.to_string())?;
    Ok(summary)
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
