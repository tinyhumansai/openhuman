use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use super::policy::{classify_urgency, recommended_start_at, reminder_plan, strip_html_summary};
use super::types::{approved_course_matchers, CanvasTask, LocalStatus, UrgencyLevel};

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

#[derive(Debug, Clone, Deserialize)]
pub struct CanvasPlannerItemDto {
    pub course_id: Option<Value>,
    pub plannable_type: Option<String>,
    pub plannable_id: Option<Value>,
    pub plannable: Option<CanvasAssignmentDto>,
}

#[async_trait::async_trait]
pub(super) trait CanvasSyncApi {
    async fn get_courses(&self) -> Result<Vec<CanvasCourseDto>, String>;
    async fn get_planner_items(
        &self,
        context_codes: Vec<String>,
    ) -> Result<Vec<CanvasPlannerItemDto>, String>;
    async fn get_assignments(&self, course_id: String) -> Result<Vec<CanvasAssignmentDto>, String>;
    async fn get_assignment(
        &self,
        course_id: String,
        assignment_id: String,
    ) -> Result<CanvasAssignmentDto, String>;
}

#[async_trait::async_trait]
impl CanvasSyncApi for super::client::CanvasClient {
    async fn get_courses(&self) -> Result<Vec<CanvasCourseDto>, String> {
        self.get_json(super::client::CanvasEndpoint::Courses)
            .await
            .map_err(|e| e.to_string())
    }

    async fn get_planner_items(
        &self,
        context_codes: Vec<String>,
    ) -> Result<Vec<CanvasPlannerItemDto>, String> {
        self.get_json(super::client::CanvasEndpoint::PlannerItems { context_codes })
            .await
            .map_err(|e| e.to_string())
    }

    async fn get_assignments(&self, course_id: String) -> Result<Vec<CanvasAssignmentDto>, String> {
        self.get_json(super::client::CanvasEndpoint::Assignments { course_id })
            .await
            .map_err(|e| e.to_string())
    }

    async fn get_assignment(
        &self,
        course_id: String,
        assignment_id: String,
    ) -> Result<CanvasAssignmentDto, String> {
        self.get_json(super::client::CanvasEndpoint::Assignment {
            course_id,
            assignment_id,
        })
        .await
        .map_err(|e| e.to_string())
    }
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
    let (due_at, due_at_unclear) = normalize_due_at(assignment.due_at);

    let mut task = CanvasTask {
        course_id: course_id.to_string(),
        course_name: course_name.to_string(),
        assignment_id: id_to_string(&assignment.id),
        assignment_name: assignment
            .name
            .unwrap_or_else(|| "Untitled assignment".to_string()),
        due_at,
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
    sync_once_with_client(&client, store, settings, now).await
}

pub(super) async fn sync_once_with_client<C: CanvasSyncApi + Sync>(
    client: &C,
    store: super::store::CanvasTrackerStore,
    _settings: &super::types::CanvasTrackerSettings,
    now: DateTime<Utc>,
) -> Result<super::types::SyncSummary, String> {
    let courses = client.get_courses().await?;
    let mut used_courses = Vec::new();
    let mut ignored = 0usize;
    let approved_allowlist = approved_course_matchers();
    for course in courses.iter() {
        let id = id_to_string(&course.id);
        let name = course.name.clone().unwrap_or_default();
        if super::policy::course_matches_allowlist(Some(&id), &name, &approved_allowlist) {
            used_courses.push((id, name));
        } else {
            ignored += 1;
        }
    }

    let mut tasks = Vec::new();
    let mut seen = HashSet::new();
    let course_names: HashMap<_, _> = used_courses.iter().cloned().collect();
    let course_ids: HashSet<_> = used_courses
        .iter()
        .map(|(course_id, _)| course_id.clone())
        .collect();
    let context_codes: Vec<_> = used_courses
        .iter()
        .map(|(course_id, _)| format!("course_{course_id}"))
        .collect();

    if !context_codes.is_empty() {
        let planner_items = client.get_planner_items(context_codes).await?;
        for item in planner_items {
            if !is_assignment_planner_item(&item) {
                continue;
            }
            let Some(course_id) = item
                .course_id
                .as_ref()
                .map(id_to_string)
                .filter(|id| course_ids.contains(id))
            else {
                continue;
            };
            let Some(course_name) = course_names.get(&course_id) else {
                continue;
            };
            if let Some(assignment) = item.plannable {
                push_unique_task(
                    &mut tasks,
                    &mut seen,
                    &course_id,
                    course_name,
                    assignment,
                    now,
                );
            } else if let Some(assignment_id) = planner_assignment_id(&item) {
                if seen.contains(&(course_id.clone(), assignment_id.clone())) {
                    continue;
                }
                let assignment = client
                    .get_assignment(course_id.clone(), assignment_id)
                    .await?;
                push_unique_task(
                    &mut tasks,
                    &mut seen,
                    &course_id,
                    course_name,
                    assignment,
                    now,
                );
            }
        }
    }

    for (course_id, course_name) in used_courses.iter() {
        let assignments = client.get_assignments(course_id.clone()).await?;
        for assignment in assignments {
            push_unique_task(
                &mut tasks,
                &mut seen,
                course_id,
                course_name,
                assignment,
                now,
            );
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

fn normalize_due_at(value: Option<String>) -> (Option<String>, bool) {
    let Some(raw) = value else {
        return (None, true);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return (None, true);
    }
    if DateTime::parse_from_rfc3339(trimmed).is_ok() {
        (Some(trimmed.to_string()), false)
    } else {
        (None, true)
    }
}

fn push_unique_task(
    tasks: &mut Vec<CanvasTask>,
    seen: &mut HashSet<(String, String)>,
    course_id: &str,
    course_name: &str,
    assignment: CanvasAssignmentDto,
    now: DateTime<Utc>,
) {
    let assignment_id = id_to_string(&assignment.id);
    if seen.insert((course_id.to_string(), assignment_id)) {
        tasks.push(normalize_assignment(
            course_id,
            course_name,
            assignment,
            now,
        ));
    }
}

fn is_assignment_planner_item(item: &CanvasPlannerItemDto) -> bool {
    item.plannable_type
        .as_deref()
        .map(|value| value.eq_ignore_ascii_case("assignment"))
        .unwrap_or_else(|| item.plannable.is_some())
}

fn planner_assignment_id(item: &CanvasPlannerItemDto) -> Option<String> {
    item.plannable
        .as_ref()
        .map(|assignment| id_to_string(&assignment.id))
        .or_else(|| item.plannable_id.as_ref().map(id_to_string))
        .filter(|id| !id.is_empty())
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
