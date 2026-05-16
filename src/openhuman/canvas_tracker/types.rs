use serde::{Deserialize, Serialize};

pub const CANVAS_TRACKER_PROVIDER: &str = "canvas-lms";
pub const DEFAULT_CANVAS_HOST: &str = "https://mango-cmu.instructure.com";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourseMatcher {
    pub canvas_id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasTrackerSettings {
    pub enabled: bool,
    pub host: String,
    pub allowlisted_courses: Vec<CourseMatcher>,
    pub token_set: bool,
}

pub fn approved_course_matchers() -> Vec<CourseMatcher> {
    vec![
        CourseMatcher {
            canvas_id: None,
            name: "361100-Secrets of the Soil-Lec.001 | 801[3/68]".to_string(),
        },
        CourseMatcher {
            canvas_id: None,
            name: "515101-Radiation in Everyday Life-Lec.002[3/68]".to_string(),
        },
    ]
}

impl CanvasTrackerSettings {
    pub fn enforce_approved_allowlist(&mut self) {
        self.allowlisted_courses = approved_course_matchers();
    }
}

impl Default for CanvasTrackerSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            host: DEFAULT_CANVAS_HOST.to_string(),
            allowlisted_courses: approved_course_matchers(),
            token_set: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalStatus {
    NotStarted,
    InProgress,
    Waiting,
    Submitted,
    Done,
    Unclear,
}

impl LocalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::InProgress => "in_progress",
            Self::Waiting => "waiting",
            Self::Submitted => "submitted",
            Self::Done => "done",
            Self::Unclear => "unclear",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UrgencyLevel {
    Critical,
    High,
    Medium,
    Low,
    Unclear,
}

impl UrgencyLevel {
    pub fn rank(self) -> u8 {
        match self {
            Self::Critical => 0,
            Self::High => 1,
            Self::Medium => 2,
            Self::Unclear => 3,
            Self::Low => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderRecommendation {
    pub kind: String,
    pub at: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanvasTask {
    pub course_id: String,
    pub course_name: String,
    pub assignment_id: String,
    pub assignment_name: String,
    pub due_at: Option<String>,
    pub due_at_unclear: bool,
    pub instructions_summary: String,
    pub submission_type: Option<String>,
    pub canvas_workflow_state: Option<String>,
    pub canvas_submission_state: Option<String>,
    pub local_status: LocalStatus,
    pub urgency_level: UrgencyLevel,
    pub recommended_start_at: Option<String>,
    pub reminders_needed: Vec<ReminderRecommendation>,
    pub source_url: Option<String>,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSummary {
    pub synced: bool,
    pub courses_seen: usize,
    pub courses_used: usize,
    pub courses_ignored: usize,
    pub assignments_seen: usize,
    pub tasks_upserted: usize,
    pub previous_tasks_preserved: bool,
    pub errors: Vec<String>,
    pub synced_at: String,
}
