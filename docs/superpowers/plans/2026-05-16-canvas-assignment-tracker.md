# Canvas Assignment Tracker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a read-only Canvas LMS tracker for the two approved current courses, with local task status, urgency, start dates, and reminder recommendations.

**Architecture:** Put Canvas business logic in a new Rust core domain at `src/openhuman/canvas_tracker/`; React only calls JSON-RPC wrappers and renders the result. The Rust domain owns the Canvas allowlist, read-only HTTP policy, token storage, SQLite persistence, sync normalization, and controller surface.

**Tech Stack:** Rust 2021, Tokio, reqwest, rusqlite, chrono, serde, existing OpenHuman JSON-RPC controller registry, React 19, TypeScript, Vitest, Testing Library.

---

## File Structure

- Create `src/openhuman/canvas_tracker/mod.rs`: domain exports and test module wiring.
- Create `src/openhuman/canvas_tracker/types.rs`: serializable settings, task, status, urgency, sync, and reminder types.
- Create `src/openhuman/canvas_tracker/policy.rs`: pure allowlist, due-date, summary, urgency, reminder, and safety helpers.
- Create `src/openhuman/canvas_tracker/policy_tests.rs`: pure Rust tests for rules that do not need I/O.
- Create `src/openhuman/canvas_tracker/auth.rs`: Canvas token storage using `AuthService`.
- Create `src/openhuman/canvas_tracker/store.rs`: SQLite persistence under `<workspace>/canvas_tracker/canvas_tracker.db`.
- Create `src/openhuman/canvas_tracker/store_tests.rs`: store round-trip and status-preservation tests.
- Create `src/openhuman/canvas_tracker/client.rs`: Canvas HTTP client that can only issue GET requests to allowed endpoint families on the configured host.
- Create `src/openhuman/canvas_tracker/client_tests.rs`: URL policy and redaction tests.
- Create `src/openhuman/canvas_tracker/sync.rs`: manual sync orchestration and normalization from Canvas payloads into local tasks.
- Create `src/openhuman/canvas_tracker/sync_tests.rs`: mock Canvas sync tests covering ignored courses and failure behavior.
- Create `src/openhuman/canvas_tracker/ops.rs`: async operations used by RPC handlers.
- Create `src/openhuman/canvas_tracker/schemas.rs`: JSON-RPC schemas and handlers.
- Modify `src/openhuman/mod.rs`: expose `canvas_tracker`.
- Modify `src/core/all.rs`: register Canvas tracker controllers and declared schemas.
- Create `app/src/lib/canvasTracker/types.ts`: frontend wire types.
- Create `app/src/lib/canvasTracker/canvasTrackerApi.ts`: typed RPC wrapper.
- Create `app/src/lib/canvasTracker/canvasTrackerApi.test.ts`: RPC envelope and token-redaction tests.
- Create `app/src/lib/canvasTracker/hooks.ts`: page data hook.
- Create `app/src/pages/CanvasTracker.tsx`: tracker page.
- Create `app/src/pages/__tests__/CanvasTracker.test.tsx`: UI tests for token hiding, sorting, status update, and errors.
- Modify `app/src/AppRoutes.tsx`: add `/canvas-tracker`.
- Modify `app/src/components/BottomTabBar.tsx`: add a compact Canvas tab.
- Modify `app/src/lib/i18n/en.ts`, `app/src/lib/i18n/id.ts`, `app/src/lib/i18n/zh-CN.ts`: add `nav.canvasTracker`.

## Task 1: Core Types And Pure Policy

**Files:**
- Create: `src/openhuman/canvas_tracker/mod.rs`
- Create: `src/openhuman/canvas_tracker/types.rs`
- Create: `src/openhuman/canvas_tracker/policy.rs`
- Create: `src/openhuman/canvas_tracker/policy_tests.rs`
- Modify: `src/openhuman/mod.rs`

- [ ] **Step 1: Create failing policy tests**

Create `src/openhuman/canvas_tracker/policy_tests.rs` with:

```rust
use chrono::{TimeZone, Utc};

use super::policy::{
    classify_urgency, course_matches_allowlist, recommended_start_at, reminder_plan,
    strip_html_summary,
};
use super::types::{CanvasTask, CourseMatcher, LocalStatus, UrgencyLevel};

fn task_due_at(due_at: Option<&str>, status: LocalStatus) -> CanvasTask {
    CanvasTask {
        course_id: "101".to_string(),
        course_name: "361100-Secrets of the Soil-Lec.001 | 801[3/68]".to_string(),
        assignment_id: "55".to_string(),
        assignment_name: "Soil reflection".to_string(),
        due_at: due_at.map(str::to_string),
        due_at_unclear: due_at.is_none(),
        instructions_summary: "Write one page.".to_string(),
        submission_type: Some("online_upload".to_string()),
        canvas_workflow_state: Some("published".to_string()),
        canvas_submission_state: None,
        local_status: status,
        urgency_level: UrgencyLevel::Unclear,
        recommended_start_at: None,
        reminders_needed: vec![],
        source_url: Some("/courses/101/assignments/55".to_string()),
        last_seen_at: "2026-05-16T06:00:00Z".to_string(),
    }
}

#[test]
fn allowlist_matches_only_exact_or_prefix_course_names() {
    let matchers = vec![
        CourseMatcher {
            canvas_id: None,
            name: "361100-Secrets of the Soil-Lec.001 | 801[3/68]".to_string(),
        },
        CourseMatcher {
            canvas_id: Some("202".to_string()),
            name: "515101-Radiation in Everyday Life-Lec.002[3/68]".to_string(),
        },
    ];

    assert!(course_matches_allowlist(
        Some("101"),
        "361100-Secrets of the Soil-Lec.001 | 801[3/68]",
        &matchers
    ));
    assert!(course_matches_allowlist(
        Some("202"),
        "Any full API name for radiation",
        &matchers
    ));
    assert!(!course_matches_allowlist(
        Some("303"),
        "001201 - CRIT READ AND EFFEC WRITE",
        &matchers
    ));
}

#[test]
fn unclear_due_date_stays_unclear_instead_of_guessing() {
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();
    let task = task_due_at(None, LocalStatus::NotStarted);

    assert_eq!(classify_urgency(&task, now), UrgencyLevel::Unclear);
    assert_eq!(recommended_start_at(&task, now), None);
}

#[test]
fn urgency_uses_due_window_and_ignores_done_tasks() {
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();

    assert_eq!(
        classify_urgency(
            &task_due_at(Some("2026-05-17T05:00:00Z"), LocalStatus::NotStarted),
            now
        ),
        UrgencyLevel::Critical
    );
    assert_eq!(
        classify_urgency(
            &task_due_at(Some("2026-05-18T06:00:00Z"), LocalStatus::InProgress),
            now
        ),
        UrgencyLevel::High
    );
    assert_eq!(
        classify_urgency(
            &task_due_at(Some("2026-05-23T06:00:00Z"), LocalStatus::Done),
            now
        ),
        UrgencyLevel::Low
    );
}

#[test]
fn summary_strips_html_and_keeps_deliverable_text() {
    let html = "<p><strong>Submit</strong> a PDF report.</p><p>Use Canvas upload.</p>";
    assert_eq!(
        strip_html_summary(html),
        "Submit a PDF report. Use Canvas upload."
    );
}

#[test]
fn reminder_plan_includes_immediate_alert_for_not_started_critical_work() {
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();
    let task = task_due_at(Some("2026-05-17T05:00:00Z"), LocalStatus::NotStarted);
    let reminders = reminder_plan(&task, now);

    assert!(reminders.iter().any(|r| r.kind == "not_started_due_soon"));
    assert!(reminders.iter().any(|r| r.kind == "due_24h"));
}
```

- [ ] **Step 2: Run the tests and verify they fail**

Run:

```bash
cargo test --manifest-path Cargo.toml canvas_tracker::policy_tests -- --nocapture
```

Expected: FAIL because `src/openhuman/canvas_tracker/*` does not exist.

- [ ] **Step 3: Create the domain module**

Create `src/openhuman/canvas_tracker/mod.rs`:

```rust
pub mod auth;
pub mod client;
pub mod ops;
pub mod policy;
mod schemas;
pub mod store;
pub mod sync;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_canvas_tracker_controller_schemas,
    all_registered_controllers as all_canvas_tracker_registered_controllers,
};

#[cfg(test)]
#[path = "policy_tests.rs"]
mod policy_tests;
#[cfg(test)]
#[path = "store_tests.rs"]
mod store_tests;
#[cfg(test)]
#[path = "client_tests.rs"]
mod client_tests;
#[cfg(test)]
#[path = "sync_tests.rs"]
mod sync_tests;
```

Modify `src/openhuman/mod.rs` by adding this public module beside the other domain modules:

```rust
pub mod canvas_tracker;
```

- [ ] **Step 4: Create serializable tracker types**

Create `src/openhuman/canvas_tracker/types.rs`:

```rust
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

impl Default for CanvasTrackerSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            host: DEFAULT_CANVAS_HOST.to_string(),
            allowlisted_courses: vec![
                CourseMatcher {
                    canvas_id: None,
                    name: "361100-Secrets of the Soil-Lec.001 | 801[3/68]".to_string(),
                },
                CourseMatcher {
                    canvas_id: None,
                    name: "515101-Radiation in Everyday Life-Lec.002[3/68]".to_string(),
                },
            ],
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
```

- [ ] **Step 5: Create pure policy helpers**

Create `src/openhuman/canvas_tracker/policy.rs`:

```rust
use chrono::{DateTime, Duration, Utc};
use regex::Regex;

use super::types::{CanvasTask, CourseMatcher, LocalStatus, ReminderRecommendation, UrgencyLevel};

pub fn normalize_course_name(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn course_matches_allowlist(
    canvas_id: Option<&str>,
    course_name: &str,
    allowlist: &[CourseMatcher],
) -> bool {
    let normalized = normalize_course_name(course_name);
    allowlist.iter().any(|matcher| {
        let id_matches = matcher
            .canvas_id
            .as_deref()
            .zip(canvas_id)
            .map(|(expected, actual)| expected == actual)
            .unwrap_or(false);
        let matcher_name = normalize_course_name(&matcher.name);
        id_matches || normalized == matcher_name || normalized.starts_with(&matcher_name)
    })
}

pub fn strip_html_summary(html: &str) -> String {
    let without_tags = Regex::new(r"(?is)<[^>]+>")
        .expect("valid tag regex")
        .replace_all(html, " ");
    let decoded = without_tags
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_due_at(value: &Option<String>) -> Option<DateTime<Utc>> {
    value
        .as_deref()
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

fn is_finished(status: LocalStatus) -> bool {
    matches!(status, LocalStatus::Submitted | LocalStatus::Done)
}

pub fn classify_urgency(task: &CanvasTask, now: DateTime<Utc>) -> UrgencyLevel {
    if task.due_at_unclear {
        return UrgencyLevel::Unclear;
    }
    let Some(due) = parse_due_at(&task.due_at) else {
        return UrgencyLevel::Unclear;
    };
    if is_finished(task.local_status) {
        return UrgencyLevel::Low;
    }
    let remaining = due - now;
    if remaining <= Duration::hours(24) {
        UrgencyLevel::Critical
    } else if remaining <= Duration::days(3) {
        UrgencyLevel::High
    } else if remaining <= Duration::days(7) {
        UrgencyLevel::Medium
    } else {
        UrgencyLevel::Low
    }
}

pub fn recommended_start_at(task: &CanvasTask, now: DateTime<Utc>) -> Option<String> {
    if task.due_at_unclear {
        return None;
    }
    let due = parse_due_at(&task.due_at)?;
    let urgency = classify_urgency(task, now);
    let start = match urgency {
        UrgencyLevel::Critical => now,
        UrgencyLevel::High => now,
        UrgencyLevel::Medium | UrgencyLevel::Low => {
            let lower = task.instructions_summary.to_ascii_lowercase();
            let complex = ["project", "presentation", "group", "quiz", "upload", "file"]
                .iter()
                .any(|needle| lower.contains(needle));
            due - if complex { Duration::days(4) } else { Duration::days(2) }
        }
        UrgencyLevel::Unclear => return None,
    };
    Some(start.to_rfc3339())
}

pub fn reminder_plan(task: &CanvasTask, now: DateTime<Utc>) -> Vec<ReminderRecommendation> {
    if task.due_at_unclear {
        return vec![ReminderRecommendation {
            kind: "due_unclear".to_string(),
            at: None,
            message: "Due date is unclear; check Canvas manually.".to_string(),
        }];
    }

    let Some(due) = parse_due_at(&task.due_at) else {
        return vec![ReminderRecommendation {
            kind: "due_unclear".to_string(),
            at: None,
            message: "Due date is unclear; check Canvas manually.".to_string(),
        }];
    };

    let mut reminders = vec![
        ReminderRecommendation {
            kind: "due_3d".to_string(),
            at: Some((due - Duration::days(3)).to_rfc3339()),
            message: "Assignment is due in 3 days.".to_string(),
        },
        ReminderRecommendation {
            kind: "due_24h".to_string(),
            at: Some((due - Duration::hours(24)).to_rfc3339()),
            message: "Assignment is due in 24 hours.".to_string(),
        },
        ReminderRecommendation {
            kind: "due_morning".to_string(),
            at: Some(due.date_naive().and_hms_opt(8, 0, 0).unwrap().and_utc().to_rfc3339()),
            message: "Assignment is due today.".to_string(),
        },
    ];

    if task.local_status == LocalStatus::NotStarted && due - now <= Duration::hours(24) {
        reminders.push(ReminderRecommendation {
            kind: "not_started_due_soon".to_string(),
            at: Some(now.to_rfc3339()),
            message: "Not started and due within 24 hours.".to_string(),
        });
    }

    reminders
}
```

- [ ] **Step 6: Run policy tests**

Run:

```bash
cargo test --manifest-path Cargo.toml canvas_tracker::policy_tests -- --nocapture
```

Expected: PASS.

- [ ] **Step 7: Commit Task 1**

```bash
git add src/openhuman/mod.rs src/openhuman/canvas_tracker/mod.rs src/openhuman/canvas_tracker/types.rs src/openhuman/canvas_tracker/policy.rs src/openhuman/canvas_tracker/policy_tests.rs
git commit -m "feat(canvas): add tracker policy types"
```

## Task 2: Local Store And Token Helpers

**Files:**
- Create: `src/openhuman/canvas_tracker/auth.rs`
- Create: `src/openhuman/canvas_tracker/store.rs`
- Create: `src/openhuman/canvas_tracker/store_tests.rs`

- [ ] **Step 1: Write failing store tests**

Create `src/openhuman/canvas_tracker/store_tests.rs`:

```rust
use tempfile::tempdir;

use super::store::CanvasTrackerStore;
use super::types::{CanvasTask, LocalStatus, UrgencyLevel};

fn task(id: &str, status: LocalStatus) -> CanvasTask {
    CanvasTask {
        course_id: "101".into(),
        course_name: "361100-Secrets of the Soil-Lec.001 | 801[3/68]".into(),
        assignment_id: id.into(),
        assignment_name: format!("Assignment {id}"),
        due_at: Some("2026-05-20T06:00:00Z".into()),
        due_at_unclear: false,
        instructions_summary: "Submit a PDF.".into(),
        submission_type: Some("online_upload".into()),
        canvas_workflow_state: Some("published".into()),
        canvas_submission_state: None,
        local_status: status,
        urgency_level: UrgencyLevel::Medium,
        recommended_start_at: Some("2026-05-18T06:00:00Z".into()),
        reminders_needed: vec![],
        source_url: Some("/courses/101/assignments/55".into()),
        last_seen_at: "2026-05-16T06:00:00Z".into(),
    }
}

#[test]
fn settings_round_trip_uses_defaults() {
    let temp = tempdir().unwrap();
    let store = CanvasTrackerStore::new(temp.path()).unwrap();
    let settings = store.get_settings().unwrap();

    assert!(settings.enabled);
    assert_eq!(settings.allowlisted_courses.len(), 2);
    assert_eq!(settings.host, "https://mango-cmu.instructure.com");
}

#[test]
fn task_upsert_preserves_existing_local_status() {
    let temp = tempdir().unwrap();
    let store = CanvasTrackerStore::new(temp.path()).unwrap();

    store.upsert_tasks(&[task("55", LocalStatus::NotStarted)]).unwrap();
    store.update_local_status("101", "55", LocalStatus::InProgress).unwrap();
    store.upsert_tasks(&[task("55", LocalStatus::NotStarted)]).unwrap();

    let rows = store.list_tasks().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].local_status, LocalStatus::InProgress);
}
```

- [ ] **Step 2: Run store tests and verify they fail**

Run:

```bash
cargo test --manifest-path Cargo.toml canvas_tracker::store_tests -- --nocapture
```

Expected: FAIL because store implementation is missing.

- [ ] **Step 3: Implement credential helper**

Create `src/openhuman/canvas_tracker/auth.rs`:

```rust
use std::collections::HashMap;

use crate::openhuman::config::Config;
use crate::openhuman::credentials::{AuthService, DEFAULT_AUTH_PROFILE_NAME};
use crate::rpc::RpcOutcome;

use super::types::CANVAS_TRACKER_PROVIDER;

pub async fn store_canvas_token(
    config: &Config,
    token: &str,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err("canvas token must not be empty".to_string());
    }
    tracing::debug!(len = trimmed.len(), "[canvas_tracker] storing token (redacted)");
    let auth = AuthService::from_config(config);
    auth.store_provider_token(
        CANVAS_TRACKER_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
        trimmed,
        HashMap::new(),
        true,
    )
    .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::json!({ "stored": true }),
        "canvas token stored",
    ))
}

pub fn get_canvas_token(config: &Config) -> Result<Option<String>, String> {
    let auth = AuthService::from_config(config);
    auth.get_provider_bearer_token(CANVAS_TRACKER_PROVIDER, None)
        .map(|value| value.map(|token| token.trim().to_string()).filter(|token| !token.is_empty()))
        .map_err(|e| e.to_string())
}

pub async fn clear_canvas_token(config: &Config) -> Result<RpcOutcome<serde_json::Value>, String> {
    let auth = AuthService::from_config(config);
    let removed = auth
        .remove_profile(CANVAS_TRACKER_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::json!({ "removed": removed }),
        "canvas token cleared",
    ))
}
```

- [ ] **Step 4: Implement SQLite store**

Create `src/openhuman/canvas_tracker/store.rs` with a `CanvasTrackerStore` that opens `<workspace>/canvas_tracker/canvas_tracker.db`, runs idempotent DDL, stores default settings as JSON, and upserts tasks with status preservation.

Use this DDL exactly:

```rust
const SCHEMA_DDL: &str = "
    PRAGMA journal_mode = WAL;
    PRAGMA foreign_keys = ON;

    CREATE TABLE IF NOT EXISTS canvas_tracker_settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS canvas_tracker_tasks (
        course_id               TEXT NOT NULL,
        assignment_id           TEXT NOT NULL,
        course_name             TEXT NOT NULL,
        assignment_name         TEXT NOT NULL,
        due_at                  TEXT,
        due_at_unclear          INTEGER NOT NULL DEFAULT 0,
        instructions_summary    TEXT NOT NULL DEFAULT '',
        submission_type         TEXT,
        canvas_workflow_state   TEXT,
        canvas_submission_state TEXT,
        local_status            TEXT NOT NULL DEFAULT 'not_started',
        urgency_level           TEXT NOT NULL DEFAULT 'unclear',
        recommended_start_at    TEXT,
        reminders_needed_json   TEXT NOT NULL DEFAULT '[]',
        source_url              TEXT,
        last_seen_at            TEXT NOT NULL,
        PRIMARY KEY (course_id, assignment_id)
    );
    CREATE INDEX IF NOT EXISTS idx_canvas_tracker_tasks_due
        ON canvas_tracker_tasks(due_at);

    CREATE TABLE IF NOT EXISTS canvas_tracker_sync_runs (
        id                       TEXT PRIMARY KEY,
        synced                   INTEGER NOT NULL,
        courses_seen             INTEGER NOT NULL,
        courses_used             INTEGER NOT NULL,
        courses_ignored          INTEGER NOT NULL,
        assignments_seen         INTEGER NOT NULL,
        tasks_upserted           INTEGER NOT NULL,
        previous_tasks_preserved INTEGER NOT NULL,
        errors_json              TEXT NOT NULL DEFAULT '[]',
        synced_at                TEXT NOT NULL
    );
";
```

Core methods to implement:

```rust
impl CanvasTrackerStore {
    pub fn new(workspace_dir: &Path) -> anyhow::Result<Self>;
    pub fn get_settings(&self) -> anyhow::Result<CanvasTrackerSettings>;
    pub fn save_settings(&self, settings: &CanvasTrackerSettings) -> anyhow::Result<()>;
    pub fn upsert_tasks(&self, tasks: &[CanvasTask]) -> anyhow::Result<usize>;
    pub fn list_tasks(&self) -> anyhow::Result<Vec<CanvasTask>>;
    pub fn update_local_status(
        &self,
        course_id: &str,
        assignment_id: &str,
        status: LocalStatus,
    ) -> anyhow::Result<()>;
    pub fn record_sync_run(&self, summary: &SyncSummary) -> anyhow::Result<()>;
}
```

The `upsert_tasks` SQL must keep an existing local status:

```sql
local_status = canvas_tracker_tasks.local_status
```

unless the incoming task status is `submitted`; in that case persist `submitted`.

- [ ] **Step 5: Run store tests**

Run:

```bash
cargo test --manifest-path Cargo.toml canvas_tracker::store_tests -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit Task 2**

```bash
git add src/openhuman/canvas_tracker/auth.rs src/openhuman/canvas_tracker/store.rs src/openhuman/canvas_tracker/store_tests.rs
git commit -m "feat(canvas): persist tracker state locally"
```

## Task 3: Read-Only Canvas Client And Sync

**Files:**
- Create: `src/openhuman/canvas_tracker/client.rs`
- Create: `src/openhuman/canvas_tracker/client_tests.rs`
- Create: `src/openhuman/canvas_tracker/sync.rs`
- Create: `src/openhuman/canvas_tracker/sync_tests.rs`

- [ ] **Step 1: Write failing client policy tests**

Create `src/openhuman/canvas_tracker/client_tests.rs`:

```rust
use super::client::{CanvasEndpoint, CanvasRequestPolicy};

#[test]
fn request_policy_accepts_only_configured_canvas_host() {
    let policy = CanvasRequestPolicy::new("https://mango-cmu.instructure.com").unwrap();

    assert!(policy
        .url_for(CanvasEndpoint::Courses)
        .unwrap()
        .as_str()
        .starts_with("https://mango-cmu.instructure.com/api/v1/courses"));
    assert!(CanvasRequestPolicy::new("https://evil.example.com").is_ok());
    assert!(policy.validate_url("https://mango-cmu.instructure.com/api/v1/courses").is_ok());
    assert!(policy.validate_url("https://example.com/api/v1/courses").is_err());
}

#[test]
fn request_policy_rejects_disallowed_paths() {
    let policy = CanvasRequestPolicy::new("https://mango-cmu.instructure.com").unwrap();

    assert!(policy
        .validate_url("https://mango-cmu.instructure.com/api/v1/courses/1/assignments")
        .is_ok());
    assert!(policy
        .validate_url("https://mango-cmu.instructure.com/api/v1/courses/1/assignments/2")
        .is_ok());
    assert!(policy
        .validate_url("https://mango-cmu.instructure.com/api/v1/courses/1/assignments/2/submissions")
        .is_err());
}
```

- [ ] **Step 2: Run client tests and verify they fail**

Run:

```bash
cargo test --manifest-path Cargo.toml canvas_tracker::client_tests -- --nocapture
```

Expected: FAIL because client implementation is missing.

- [ ] **Step 3: Implement the read-only client policy**

Create `src/openhuman/canvas_tracker/client.rs` with:

```rust
use reqwest::Url;
use serde::de::DeserializeOwned;

#[derive(Debug, Clone)]
pub enum CanvasEndpoint {
    Courses,
    PlannerItems { context_codes: Vec<String> },
    Assignments { course_id: String },
    Assignment { course_id: String, assignment_id: String },
}

#[derive(Debug, Clone)]
pub struct CanvasRequestPolicy {
    host: Url,
}

impl CanvasRequestPolicy {
    pub fn new(host: &str) -> Result<Self, String> {
        let host = Url::parse(host).map_err(|e| format!("invalid canvas host: {e}"))?;
        if host.scheme() != "https" {
            return Err("canvas host must use https".to_string());
        }
        Ok(Self { host })
    }

    pub fn url_for(&self, endpoint: CanvasEndpoint) -> Result<Url, String> {
        let mut url = self.host.clone();
        match endpoint {
            CanvasEndpoint::Courses => url.set_path("/api/v1/courses"),
            CanvasEndpoint::PlannerItems { context_codes } => {
                url.set_path("/api/v1/planner/items");
                {
                    let mut pairs = url.query_pairs_mut();
                    pairs.append_pair("filter", "incomplete_items");
                    for code in context_codes {
                        pairs.append_pair("context_codes[]", &code);
                    }
                }
            }
            CanvasEndpoint::Assignments { course_id } => {
                url.set_path(&format!("/api/v1/courses/{course_id}/assignments"));
                url.query_pairs_mut().append_pair("include[]", "submission");
            }
            CanvasEndpoint::Assignment {
                course_id,
                assignment_id,
            } => {
                url.set_path(&format!(
                    "/api/v1/courses/{course_id}/assignments/{assignment_id}"
                ));
                url.query_pairs_mut().append_pair("include[]", "submission");
            }
        }
        self.validate_url(url.as_str())?;
        Ok(url)
    }

    pub fn validate_url(&self, candidate: &str) -> Result<(), String> {
        let url = Url::parse(candidate).map_err(|e| format!("invalid canvas url: {e}"))?;
        if url.scheme() != self.host.scheme()
            || url.domain() != self.host.domain()
            || url.port_or_known_default() != self.host.port_or_known_default()
        {
            return Err("canvas url host is not allowed".to_string());
        }
        let path = url.path();
        let allowed = path == "/api/v1/courses"
            || path == "/api/v1/planner/items"
            || is_assignments_path(path);
        if !allowed {
            return Err("canvas endpoint is not allowed".to_string());
        }
        Ok(())
    }
}

fn is_assignments_path(path: &str) -> bool {
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    matches!(
        parts.as_slice(),
        ["api", "v1", "courses", _, "assignments"]
            | ["api", "v1", "courses", _, "assignments", _]
    )
}

#[derive(Clone)]
pub struct CanvasClient {
    client: reqwest::Client,
    policy: CanvasRequestPolicy,
    token: String,
}

impl CanvasClient {
    pub fn new(host: &str, token: &str) -> Result<Self, String> {
        Ok(Self {
            client: reqwest::Client::new(),
            policy: CanvasRequestPolicy::new(host)?,
            token: token.trim().to_string(),
        })
    }

    pub async fn get_json<T: DeserializeOwned>(&self, endpoint: CanvasEndpoint) -> Result<T, String> {
        let url = self.policy.url_for(endpoint)?;
        self.policy.validate_url(url.as_str())?;
        let response = self
            .client
            .get(url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| format!("canvas GET failed: {}", redact_token(&e.to_string(), &self.token)))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("canvas GET returned {status}"));
        }
        response
            .json::<T>()
            .await
            .map_err(|e| format!("canvas response decode failed: {e}"))
    }
}

pub fn redact_token(message: &str, token: &str) -> String {
    if token.is_empty() {
        return message.to_string();
    }
    message.replace(token, "[REDACTED]")
}
```

- [ ] **Step 4: Write sync tests**

Create `src/openhuman/canvas_tracker/sync_tests.rs` with unit tests for normalizing JSON objects without a real network:

```rust
use chrono::{TimeZone, Utc};
use serde_json::json;

use super::sync::{normalize_assignment, CanvasAssignmentDto};
use super::types::{CourseMatcher, LocalStatus, UrgencyLevel};

#[test]
fn normalize_assignment_extracts_required_fields() {
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();
    let dto: CanvasAssignmentDto = serde_json::from_value(json!({
        "id": 55,
        "name": "Soil reflection",
        "description": "<p>Submit a PDF reflection.</p>",
        "due_at": "2026-05-18T06:00:00Z",
        "html_url": "https://mango-cmu.instructure.com/courses/101/assignments/55",
        "workflow_state": "published",
        "submission_types": ["online_upload"],
        "submission": { "workflow_state": "unsubmitted" }
    }))
    .unwrap();

    let task = normalize_assignment("101", "361100-Secrets of the Soil-Lec.001 | 801[3/68]", dto, now);

    assert_eq!(task.assignment_id, "55");
    assert_eq!(task.instructions_summary, "Submit a PDF reflection.");
    assert_eq!(task.submission_type.as_deref(), Some("online_upload"));
    assert_eq!(task.local_status, LocalStatus::NotStarted);
    assert_eq!(task.urgency_level, UrgencyLevel::High);
}

#[test]
fn normalize_assignment_marks_missing_due_date_unclear() {
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();
    let dto: CanvasAssignmentDto = serde_json::from_value(json!({
        "id": 56,
        "name": "No due date assignment",
        "description": "<p>Read the page.</p>",
        "due_at": null,
        "submission_types": ["none"]
    }))
    .unwrap();

    let task = normalize_assignment("101", "361100-Secrets of the Soil-Lec.001 | 801[3/68]", dto, now);

    assert!(task.due_at_unclear);
    assert_eq!(task.urgency_level, UrgencyLevel::Unclear);
    assert_eq!(task.recommended_start_at, None);
}
```

- [ ] **Step 5: Implement sync DTOs and normalizer**

Create `src/openhuman/canvas_tracker/sync.rs` with DTOs and `normalize_assignment`:

```rust
use chrono::Utc;
use serde::Deserialize;

use super::policy::{classify_urgency, recommended_start_at, reminder_plan, strip_html_summary};
use super::types::{CanvasTask, LocalStatus};

#[derive(Debug, Clone, Deserialize)]
pub struct CanvasCourseDto {
    pub id: serde_json::Value,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CanvasSubmissionDto {
    pub workflow_state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CanvasAssignmentDto {
    pub id: serde_json::Value,
    pub name: String,
    pub description: Option<String>,
    pub due_at: Option<String>,
    pub html_url: Option<String>,
    pub workflow_state: Option<String>,
    pub submission_types: Option<Vec<String>>,
    pub submission: Option<CanvasSubmissionDto>,
}

fn id_to_string(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

pub fn normalize_assignment(
    course_id: &str,
    course_name: &str,
    assignment: CanvasAssignmentDto,
    now: chrono::DateTime<Utc>,
) -> CanvasTask {
    let assignment_id = id_to_string(assignment.id);
    let submission_state = assignment
        .submission
        .as_ref()
        .and_then(|submission| submission.workflow_state.clone());
    let submitted = submission_state
        .as_deref()
        .map(|state| matches!(state, "submitted" | "graded" | "pending_review"))
        .unwrap_or(false);

    let mut task = CanvasTask {
        course_id: course_id.to_string(),
        course_name: course_name.to_string(),
        assignment_id,
        assignment_name: assignment.name,
        due_at_unclear: assignment.due_at.is_none(),
        due_at: assignment.due_at,
        instructions_summary: strip_html_summary(assignment.description.as_deref().unwrap_or("")),
        submission_type: assignment
            .submission_types
            .unwrap_or_default()
            .into_iter()
            .find(|value| !value.trim().is_empty()),
        canvas_workflow_state: assignment.workflow_state,
        canvas_submission_state: submission_state,
        local_status: if submitted {
            LocalStatus::Submitted
        } else {
            LocalStatus::NotStarted
        },
        urgency_level: super::types::UrgencyLevel::Unclear,
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
```

- [ ] **Step 6: Run client and sync tests**

Run:

```bash
cargo test --manifest-path Cargo.toml canvas_tracker::client_tests canvas_tracker::sync_tests -- --nocapture
```

Expected: PASS.

- [ ] **Step 7: Commit Task 3**

```bash
git add src/openhuman/canvas_tracker/client.rs src/openhuman/canvas_tracker/client_tests.rs src/openhuman/canvas_tracker/sync.rs src/openhuman/canvas_tracker/sync_tests.rs
git commit -m "feat(canvas): add read-only canvas sync primitives"
```

## Task 4: RPC Operations And Registry

**Files:**
- Create: `src/openhuman/canvas_tracker/ops.rs`
- Create: `src/openhuman/canvas_tracker/schemas.rs`
- Modify: `src/core/all.rs`

- [ ] **Step 1: Write schema registration tests**

Append this test module to `src/openhuman/canvas_tracker/schemas.rs` after the handlers are created in Step 3:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_tracker_schemas_have_expected_namespace() {
        let schemas = all_controller_schemas();
        let names: Vec<String> = schemas
            .iter()
            .map(|schema| format!("{}.{}", schema.namespace, schema.function))
            .collect();

        assert!(names.contains(&"canvas_tracker.get_settings".to_string()));
        assert!(names.contains(&"canvas_tracker.update_settings".to_string()));
        assert!(names.contains(&"canvas_tracker.sync_now".to_string()));
        assert!(names.contains(&"canvas_tracker.list_tasks".to_string()));
        assert!(names.contains(&"canvas_tracker.update_local_status".to_string()));
        assert!(names.contains(&"canvas_tracker.list_reminders".to_string()));
    }

    #[test]
    fn schema_and_handler_counts_match() {
        assert_eq!(
            all_controller_schemas().len(),
            all_registered_controllers().len()
        );
    }
}
```

- [ ] **Step 2: Run schema tests and verify they fail**

Run:

```bash
cargo test --manifest-path Cargo.toml canvas_tracker::schemas::tests -- --nocapture
```

Expected: FAIL because schemas are not implemented.

- [ ] **Step 3: Implement operations**

Create `src/openhuman/canvas_tracker/ops.rs` with operations that load config, never expose the token, and use the store:

```rust
use chrono::Utc;

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::auth::{clear_canvas_token, get_canvas_token, store_canvas_token};
use super::store::CanvasTrackerStore;
use super::types::{CanvasTask, CanvasTrackerSettings, LocalStatus, SyncSummary};

pub async fn get_settings(config: &Config) -> Result<RpcOutcome<CanvasTrackerSettings>, String> {
    let store = CanvasTrackerStore::new(&config.workspace_dir).map_err(|e| e.to_string())?;
    let mut settings = store.get_settings().map_err(|e| e.to_string())?;
    settings.token_set = get_canvas_token(config)?.is_some();
    Ok(RpcOutcome::single_log(settings, "canvas tracker settings loaded"))
}

pub async fn update_settings(
    config: &Config,
    mut settings: CanvasTrackerSettings,
    token: Option<String>,
    clear_token: bool,
) -> Result<RpcOutcome<CanvasTrackerSettings>, String> {
    let store = CanvasTrackerStore::new(&config.workspace_dir).map_err(|e| e.to_string())?;
    if clear_token {
        clear_canvas_token(config).await?;
    }
    if let Some(token) = token {
        store_canvas_token(config, &token).await?;
    }
    settings.token_set = get_canvas_token(config)?.is_some();
    store.save_settings(&settings).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(settings, "canvas tracker settings saved"))
}

pub async fn list_tasks(config: &Config) -> Result<RpcOutcome<Vec<CanvasTask>>, String> {
    let store = CanvasTrackerStore::new(&config.workspace_dir).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        store.list_tasks().map_err(|e| e.to_string())?,
        "canvas tracker tasks loaded",
    ))
}

pub async fn update_local_status(
    config: &Config,
    course_id: &str,
    assignment_id: &str,
    status: LocalStatus,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let store = CanvasTrackerStore::new(&config.workspace_dir).map_err(|e| e.to_string())?;
    store
        .update_local_status(course_id, assignment_id, status)
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        serde_json::json!({ "updated": true }),
        "canvas tracker local status updated",
    ))
}

pub async fn sync_now(config: &Config) -> Result<RpcOutcome<SyncSummary>, String> {
    let store = CanvasTrackerStore::new(&config.workspace_dir).map_err(|e| e.to_string())?;
    let settings = store.get_settings().map_err(|e| e.to_string())?;
    let token = get_canvas_token(config)?.ok_or_else(|| "canvas token is not configured".to_string())?;
    let summary = super::sync::sync_once(&store, &settings, &token, Utc::now()).await?;
    Ok(RpcOutcome::single_log(summary, "canvas tracker sync complete"))
}

pub async fn list_reminders(
    config: &Config,
) -> Result<RpcOutcome<Vec<super::types::ReminderRecommendation>>, String> {
    let store = CanvasTrackerStore::new(&config.workspace_dir).map_err(|e| e.to_string())?;
    let reminders = store
        .list_tasks()
        .map_err(|e| e.to_string())?
        .into_iter()
        .flat_map(|task| task.reminders_needed)
        .collect();
    Ok(RpcOutcome::single_log(reminders, "canvas tracker reminders loaded"))
}
```

Complete `sync_once` in `src/openhuman/canvas_tracker/sync.rs` so `ops::sync_now` compiles:

```rust
pub async fn sync_once(
    store: &super::store::CanvasTrackerStore,
    settings: &super::types::CanvasTrackerSettings,
    token: &str,
    now: chrono::DateTime<Utc>,
) -> Result<super::types::SyncSummary, String> {
    let client = super::client::CanvasClient::new(&settings.host, token)?;
    let courses: Vec<CanvasCourseDto> = client
        .get_json(super::client::CanvasEndpoint::Courses)
        .await?;
    let mut used_courses = Vec::new();
    let mut ignored = 0usize;
    for course in courses.iter() {
        let id = id_to_string(course.id.clone());
        let name = course.name.clone().unwrap_or_default();
        if super::policy::course_matches_allowlist(Some(&id), &name, &settings.allowlisted_courses) {
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
            .await?;
        for assignment in assignments {
            tasks.push(normalize_assignment(course_id, course_name, assignment, now));
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
```

- [ ] **Step 4: Implement schemas and handlers**

Create `src/openhuman/canvas_tracker/schemas.rs` using the same `ControllerSchema` pattern as existing domains. The handlers must deserialize params and call `config_rpc::load_config_with_timeout().await`.

Required schema functions:

```rust
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("get_settings"),
        schemas("update_settings"),
        schemas("sync_now"),
        schemas("list_tasks"),
        schemas("update_local_status"),
        schemas("list_reminders"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController { schema: schemas("get_settings"), handler: handle_get_settings },
        RegisteredController { schema: schemas("update_settings"), handler: handle_update_settings },
        RegisteredController { schema: schemas("sync_now"), handler: handle_sync_now },
        RegisteredController { schema: schemas("list_tasks"), handler: handle_list_tasks },
        RegisteredController { schema: schemas("update_local_status"), handler: handle_update_local_status },
        RegisteredController { schema: schemas("list_reminders"), handler: handle_list_reminders },
    ]
}
```

The `update_settings` params struct must include `settings`, optional `token`, and optional `clear_token`; returned settings must only include `token_set: true|false`.

- [ ] **Step 5: Register controllers globally**

Modify `src/core/all.rs` in both registry builders.

In `build_registered_controllers()`, add near other domain controllers:

```rust
controllers.extend(crate::openhuman::canvas_tracker::all_canvas_tracker_registered_controllers());
```

In `build_declared_controller_schemas()`, add:

```rust
schemas.extend(crate::openhuman::canvas_tracker::all_canvas_tracker_controller_schemas());
```

- [ ] **Step 6: Run core tests**

Run:

```bash
cargo test --manifest-path Cargo.toml canvas_tracker -- --nocapture
cargo check --manifest-path Cargo.toml
```

Expected: PASS.

- [ ] **Step 7: Commit Task 4**

```bash
git add src/core/all.rs src/openhuman/canvas_tracker/ops.rs src/openhuman/canvas_tracker/schemas.rs src/openhuman/canvas_tracker/sync.rs
git commit -m "feat(canvas): expose tracker rpc controllers"
```

## Task 5: Frontend RPC Wrapper And Hook

**Files:**
- Create: `app/src/lib/canvasTracker/types.ts`
- Create: `app/src/lib/canvasTracker/canvasTrackerApi.ts`
- Create: `app/src/lib/canvasTracker/canvasTrackerApi.test.ts`
- Create: `app/src/lib/canvasTracker/hooks.ts`

- [ ] **Step 1: Write failing API wrapper tests**

Create `app/src/lib/canvasTracker/canvasTrackerApi.test.ts`:

```ts
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { getCanvasTrackerSettings, updateCanvasTrackerSettings } from './canvasTrackerApi';

const callCoreRpc = vi.fn();

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc }));

describe('canvasTrackerApi', () => {
  beforeEach(() => {
    callCoreRpc.mockReset();
  });

  it('unwraps RpcOutcome envelopes from settings reads', async () => {
    callCoreRpc.mockResolvedValue({
      result: {
        enabled: true,
        host: 'https://mango-cmu.instructure.com',
        token_set: false,
        allowlisted_courses: [],
      },
      logs: ['ok'],
    });

    await expect(getCanvasTrackerSettings()).resolves.toMatchObject({
      enabled: true,
      token_set: false,
    });
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.canvas_tracker_get_settings',
    });
  });

  it('sends token only to update_settings and never returns it', async () => {
    callCoreRpc.mockResolvedValue({
      result: {
        enabled: true,
        host: 'https://mango-cmu.instructure.com',
        token_set: true,
        allowlisted_courses: [],
      },
      logs: [],
    });

    const result = await updateCanvasTrackerSettings({
      settings: {
        enabled: true,
        host: 'https://mango-cmu.instructure.com',
        token_set: false,
        allowlisted_courses: [],
      },
      token: 'canvas-secret-token',
    });

    expect(result.token_set).toBe(true);
    expect(JSON.stringify(result)).not.toContain('canvas-secret-token');
  });
});
```

- [ ] **Step 2: Run API wrapper tests and verify they fail**

Run:

```bash
pnpm --filter openhuman-app test -- src/lib/canvasTracker/canvasTrackerApi.test.ts
```

Expected: FAIL because files do not exist.

- [ ] **Step 3: Implement frontend types**

Create `app/src/lib/canvasTracker/types.ts`:

```ts
export type LocalStatus =
  | 'not_started'
  | 'in_progress'
  | 'waiting'
  | 'submitted'
  | 'done'
  | 'unclear';

export type UrgencyLevel = 'critical' | 'high' | 'medium' | 'low' | 'unclear';

export interface CourseMatcher {
  canvas_id?: string | null;
  name: string;
}

export interface CanvasTrackerSettings {
  enabled: boolean;
  host: string;
  allowlisted_courses: CourseMatcher[];
  token_set: boolean;
}

export interface ReminderRecommendation {
  kind: string;
  at?: string | null;
  message: string;
}

export interface CanvasTask {
  course_id: string;
  course_name: string;
  assignment_id: string;
  assignment_name: string;
  due_at?: string | null;
  due_at_unclear: boolean;
  instructions_summary: string;
  submission_type?: string | null;
  canvas_workflow_state?: string | null;
  canvas_submission_state?: string | null;
  local_status: LocalStatus;
  urgency_level: UrgencyLevel;
  recommended_start_at?: string | null;
  reminders_needed: ReminderRecommendation[];
  source_url?: string | null;
  last_seen_at: string;
}

export interface SyncSummary {
  synced: boolean;
  courses_seen: number;
  courses_used: number;
  courses_ignored: number;
  assignments_seen: number;
  tasks_upserted: number;
  previous_tasks_preserved: boolean;
  errors: string[];
  synced_at: string;
}
```

- [ ] **Step 4: Implement RPC wrapper**

Create `app/src/lib/canvasTracker/canvasTrackerApi.ts`:

```ts
import { callCoreRpc } from '../../services/coreRpcClient';
import type { CanvasTask, CanvasTrackerSettings, LocalStatus, ReminderRecommendation, SyncSummary } from './types';

function unwrapCliEnvelope<T>(value: unknown): T {
  if (
    value !== null &&
    typeof value === 'object' &&
    'result' in (value as Record<string, unknown>) &&
    'logs' in (value as Record<string, unknown>) &&
    Array.isArray((value as { logs: unknown }).logs)
  ) {
    return (value as { result: T }).result;
  }
  return value as T;
}

export async function getCanvasTrackerSettings(): Promise<CanvasTrackerSettings> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.canvas_tracker_get_settings' });
  return unwrapCliEnvelope<CanvasTrackerSettings>(raw);
}

export async function updateCanvasTrackerSettings(input: {
  settings: CanvasTrackerSettings;
  token?: string;
  clear_token?: boolean;
}): Promise<CanvasTrackerSettings> {
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.canvas_tracker_update_settings',
    params: input,
  });
  return unwrapCliEnvelope<CanvasTrackerSettings>(raw);
}

export async function syncCanvasTrackerNow(): Promise<SyncSummary> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.canvas_tracker_sync_now' });
  return unwrapCliEnvelope<SyncSummary>(raw);
}

export async function listCanvasTrackerTasks(): Promise<CanvasTask[]> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.canvas_tracker_list_tasks' });
  return unwrapCliEnvelope<CanvasTask[]>(raw);
}

export async function updateCanvasTaskStatus(input: {
  course_id: string;
  assignment_id: string;
  status: LocalStatus;
}): Promise<{ updated: boolean }> {
  const raw = await callCoreRpc<unknown>({
    method: 'openhuman.canvas_tracker_update_local_status',
    params: input,
  });
  return unwrapCliEnvelope<{ updated: boolean }>(raw);
}

export async function listCanvasTrackerReminders(): Promise<ReminderRecommendation[]> {
  const raw = await callCoreRpc<unknown>({ method: 'openhuman.canvas_tracker_list_reminders' });
  return unwrapCliEnvelope<ReminderRecommendation[]>(raw);
}
```

- [ ] **Step 5: Implement hook**

Create `app/src/lib/canvasTracker/hooks.ts`:

```ts
import { useCallback, useEffect, useMemo, useState } from 'react';

import {
  getCanvasTrackerSettings,
  listCanvasTrackerTasks,
  syncCanvasTrackerNow,
  updateCanvasTaskStatus,
} from './canvasTrackerApi';
import type { CanvasTask, CanvasTrackerSettings, LocalStatus, SyncSummary } from './types';

const urgencyRank: Record<string, number> = {
  critical: 0,
  high: 1,
  medium: 2,
  unclear: 3,
  low: 4,
};

export function sortCanvasTasks(tasks: CanvasTask[]): CanvasTask[] {
  return [...tasks].sort((a, b) => {
    const urgency = (urgencyRank[a.urgency_level] ?? 9) - (urgencyRank[b.urgency_level] ?? 9);
    if (urgency !== 0) return urgency;
    return String(a.due_at ?? '9999').localeCompare(String(b.due_at ?? '9999'));
  });
}

export function useCanvasTracker() {
  const [settings, setSettings] = useState<CanvasTrackerSettings | null>(null);
  const [tasks, setTasks] = useState<CanvasTask[]>([]);
  const [loading, setLoading] = useState(true);
  const [syncing, setSyncing] = useState(false);
  const [lastSync, setLastSync] = useState<SyncSummary | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const [nextSettings, nextTasks] = await Promise.all([
        getCanvasTrackerSettings(),
        listCanvasTrackerTasks(),
      ]);
      setSettings(nextSettings);
      setTasks(nextTasks);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const syncNow = useCallback(async () => {
    setSyncing(true);
    setError(null);
    try {
      const summary = await syncCanvasTrackerNow();
      setLastSync(summary);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSyncing(false);
    }
  }, [refresh]);

  const updateStatus = useCallback(
    async (task: CanvasTask, status: LocalStatus) => {
      await updateCanvasTaskStatus({
        course_id: task.course_id,
        assignment_id: task.assignment_id,
        status,
      });
      await refresh();
    },
    [refresh]
  );

  const sortedTasks = useMemo(() => sortCanvasTasks(tasks), [tasks]);
  return { settings, tasks: sortedTasks, loading, syncing, lastSync, error, refresh, syncNow, updateStatus };
}
```

- [ ] **Step 6: Run API tests**

Run:

```bash
pnpm --filter openhuman-app test -- src/lib/canvasTracker/canvasTrackerApi.test.ts
```

Expected: PASS.

- [ ] **Step 7: Commit Task 5**

```bash
git add app/src/lib/canvasTracker/types.ts app/src/lib/canvasTracker/canvasTrackerApi.ts app/src/lib/canvasTracker/canvasTrackerApi.test.ts app/src/lib/canvasTracker/hooks.ts
git commit -m "feat(canvas): add frontend tracker rpc client"
```

## Task 6: Canvas Tracker Page And Navigation

**Files:**
- Create: `app/src/pages/CanvasTracker.tsx`
- Create: `app/src/pages/__tests__/CanvasTracker.test.tsx`
- Modify: `app/src/AppRoutes.tsx`
- Modify: `app/src/components/BottomTabBar.tsx`
- Modify: `app/src/lib/i18n/en.ts`
- Modify: `app/src/lib/i18n/id.ts`
- Modify: `app/src/lib/i18n/zh-CN.ts`

- [ ] **Step 1: Write failing page tests**

Create `app/src/pages/__tests__/CanvasTracker.test.tsx`:

```tsx
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import CanvasTracker from '../CanvasTracker';

vi.mock('../../lib/canvasTracker/hooks', () => ({
  useCanvasTracker: vi.fn(),
}));

const useCanvasTracker = vi.mocked(await import('../../lib/canvasTracker/hooks')).useCanvasTracker;

describe('CanvasTracker', () => {
  beforeEach(() => {
    useCanvasTracker.mockReset();
  });

  it('renders allowlisted courses without showing a token', () => {
    useCanvasTracker.mockReturnValue({
      settings: {
        enabled: true,
        host: 'https://mango-cmu.instructure.com',
        token_set: true,
        allowlisted_courses: [
          { name: '361100-Secrets of the Soil-Lec.001 | 801[3/68]' },
          { name: '515101-Radiation in Everyday Life-Lec.002[3/68]' },
        ],
      },
      tasks: [],
      loading: false,
      syncing: false,
      lastSync: null,
      error: null,
      refresh: vi.fn(),
      syncNow: vi.fn(),
      updateStatus: vi.fn(),
    });

    render(<CanvasTracker />);

    expect(screen.getByText('Canvas Tracker')).toBeInTheDocument();
    expect(screen.getByText(/Secrets of the Soil/)).toBeInTheDocument();
    expect(screen.getByText(/Radiation in Everyday Life/)).toBeInTheDocument();
    expect(screen.queryByText(/secret/i)).not.toBeInTheDocument();
  });

  it('updates local status without submitting to Canvas', async () => {
    const updateStatus = vi.fn().mockResolvedValue(undefined);
    useCanvasTracker.mockReturnValue({
      settings: null,
      tasks: [
        {
          course_id: '101',
          course_name: '361100-Secrets of the Soil-Lec.001 | 801[3/68]',
          assignment_id: '55',
          assignment_name: 'Soil reflection',
          due_at: '2026-05-18T06:00:00Z',
          due_at_unclear: false,
          instructions_summary: 'Submit a PDF.',
          submission_type: 'online_upload',
          canvas_workflow_state: 'published',
          canvas_submission_state: null,
          local_status: 'not_started',
          urgency_level: 'high',
          recommended_start_at: '2026-05-16T06:00:00Z',
          reminders_needed: [],
          source_url: null,
          last_seen_at: '2026-05-16T06:00:00Z',
        },
      ],
      loading: false,
      syncing: false,
      lastSync: null,
      error: null,
      refresh: vi.fn(),
      syncNow: vi.fn(),
      updateStatus,
    });

    render(<CanvasTracker />);
    fireEvent.change(screen.getByLabelText('Status for Soil reflection'), {
      target: { value: 'in_progress' },
    });

    await waitFor(() => expect(updateStatus).toHaveBeenCalledWith(expect.any(Object), 'in_progress'));
  });
});
```

- [ ] **Step 2: Run page tests and verify they fail**

Run:

```bash
pnpm --filter openhuman-app test -- src/pages/__tests__/CanvasTracker.test.tsx
```

Expected: FAIL because `CanvasTracker` does not exist.

- [ ] **Step 3: Implement Canvas Tracker page**

Create `app/src/pages/CanvasTracker.tsx`:

```tsx
import { useMemo, useState } from 'react';

import { useCanvasTracker } from '../lib/canvasTracker/hooks';
import type { CanvasTask, LocalStatus } from '../lib/canvasTracker/types';

const statuses: LocalStatus[] = ['not_started', 'in_progress', 'waiting', 'submitted', 'done', 'unclear'];

function formatDate(value?: string | null, unclear?: boolean): string {
  if (unclear || !value) return 'unclear';
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(value));
}

function urgencyClass(level: string): string {
  if (level === 'critical') return 'bg-red-50 text-red-700 border-red-200';
  if (level === 'high') return 'bg-orange-50 text-orange-700 border-orange-200';
  if (level === 'medium') return 'bg-amber-50 text-amber-700 border-amber-200';
  if (level === 'unclear') return 'bg-stone-100 text-stone-700 border-stone-300';
  return 'bg-emerald-50 text-emerald-700 border-emerald-200';
}

function TaskRow({
  task,
  onStatus,
}: {
  task: CanvasTask;
  onStatus: (task: CanvasTask, status: LocalStatus) => void;
}) {
  return (
    <tr className="border-b border-stone-200 align-top">
      <td className="px-3 py-3 text-sm text-stone-700">{task.course_name}</td>
      <td className="px-3 py-3">
        <div className="text-sm font-semibold text-stone-900">{task.assignment_name}</div>
        <div className="mt-1 text-xs text-stone-500">{task.instructions_summary || 'No summary visible.'}</div>
      </td>
      <td className="px-3 py-3 text-sm text-stone-700">{formatDate(task.due_at, task.due_at_unclear)}</td>
      <td className="px-3 py-3 text-sm text-stone-700">{task.submission_type || 'not visible'}</td>
      <td className="px-3 py-3">
        <select
          aria-label={`Status for ${task.assignment_name}`}
          className="rounded-sm border border-stone-300 bg-white px-2 py-1 text-sm"
          value={task.local_status}
          onChange={event => onStatus(task, event.target.value as LocalStatus)}>
          {statuses.map(status => (
            <option key={status} value={status}>
              {status.replaceAll('_', ' ')}
            </option>
          ))}
        </select>
      </td>
      <td className="px-3 py-3">
        <span className={`inline-flex rounded-sm border px-2 py-1 text-xs font-semibold ${urgencyClass(task.urgency_level)}`}>
          {task.urgency_level}
        </span>
      </td>
      <td className="px-3 py-3 text-sm text-stone-700">{formatDate(task.recommended_start_at, task.due_at_unclear)}</td>
      <td className="px-3 py-3 text-xs text-stone-600">
        {task.reminders_needed.length === 0
          ? 'none'
          : task.reminders_needed.map(reminder => reminder.message).join(' ')}
      </td>
    </tr>
  );
}

export default function CanvasTracker() {
  const { settings, tasks, loading, syncing, lastSync, error, syncNow, updateStatus } = useCanvasTracker();
  const [filter, setFilter] = useState<LocalStatus | 'all'>('all');

  const visibleTasks = useMemo(
    () => (filter === 'all' ? tasks : tasks.filter(task => task.local_status === filter)),
    [filter, tasks]
  );

  return (
    <main className="h-full overflow-auto bg-stone-50 px-6 py-6 text-stone-900">
      <div className="mx-auto max-w-7xl">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <h1 className="text-2xl font-semibold">Canvas Tracker</h1>
            <p className="mt-1 text-sm text-stone-600">Read-only assignment tracking for your two approved Canvas courses.</p>
          </div>
          <button
            type="button"
            onClick={() => void syncNow()}
            disabled={syncing}
            className="rounded-sm bg-stone-900 px-4 py-2 text-sm font-semibold text-white disabled:opacity-50">
            {syncing ? 'Syncing...' : 'Sync now'}
          </button>
        </div>

        {error && <div className="mt-4 rounded-sm border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700">{error}</div>}

        <section className="mt-6 grid gap-4 md:grid-cols-3">
          <div className="rounded-sm border border-stone-200 bg-white p-4">
            <div className="text-xs font-semibold uppercase tracking-wide text-stone-500">Connection</div>
            <div className="mt-2 text-sm text-stone-800">{settings?.token_set ? 'Token saved locally' : 'Token not configured'}</div>
            <div className="mt-1 text-xs text-stone-500">{settings?.host ?? 'https://mango-cmu.instructure.com'}</div>
          </div>
          <div className="rounded-sm border border-stone-200 bg-white p-4 md:col-span-2">
            <div className="text-xs font-semibold uppercase tracking-wide text-stone-500">Allowlisted courses</div>
            <ul className="mt-2 space-y-1 text-sm text-stone-800">
              {(settings?.allowlisted_courses ?? []).map(course => (
                <li key={course.name}>{course.name}</li>
              ))}
            </ul>
          </div>
        </section>

        <section className="mt-6 rounded-sm border border-stone-200 bg-white">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-stone-200 px-4 py-3">
            <div>
              <h2 className="text-sm font-semibold">Tasks</h2>
              <p className="text-xs text-stone-500">
                {lastSync ? `Last sync ${formatDate(lastSync.synced_at)}` : 'Manual sync only'}
              </p>
            </div>
            <select
              aria-label="Filter tasks"
              className="rounded-sm border border-stone-300 bg-white px-2 py-1 text-sm"
              value={filter}
              onChange={event => setFilter(event.target.value as LocalStatus | 'all')}>
              <option value="all">all</option>
              {statuses.map(status => (
                <option key={status} value={status}>
                  {status.replaceAll('_', ' ')}
                </option>
              ))}
            </select>
          </div>

          {loading ? (
            <div className="px-4 py-8 text-sm text-stone-500">Loading Canvas tracker...</div>
          ) : visibleTasks.length === 0 ? (
            <div className="px-4 py-8 text-sm text-stone-500">No tasks found for the selected filter.</div>
          ) : (
            <div className="overflow-x-auto">
              <table className="min-w-full table-fixed">
                <thead className="bg-stone-100 text-left text-xs font-semibold uppercase text-stone-500">
                  <tr>
                    <th className="w-56 px-3 py-2">Course</th>
                    <th className="w-80 px-3 py-2">Assignment</th>
                    <th className="w-40 px-3 py-2">Due</th>
                    <th className="w-36 px-3 py-2">Submission</th>
                    <th className="w-40 px-3 py-2">Status</th>
                    <th className="w-28 px-3 py-2">Urgency</th>
                    <th className="w-40 px-3 py-2">Start</th>
                    <th className="w-64 px-3 py-2">Reminders</th>
                  </tr>
                </thead>
                <tbody>
                  {visibleTasks.map(task => (
                    <TaskRow key={`${task.course_id}:${task.assignment_id}`} task={task} onStatus={updateStatus} />
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>
      </div>
    </main>
  );
}
```

- [ ] **Step 4: Add route and nav**

Modify `app/src/AppRoutes.tsx`:

```tsx
import CanvasTracker from './pages/CanvasTracker';
```

Add a protected route before settings:

```tsx
<Route
  path="/canvas-tracker"
  element={
    <ProtectedRoute requireAuth={true}>
      <CanvasTracker />
    </ProtectedRoute>
  }
/>
```

Modify `app/src/components/BottomTabBar.tsx` by adding a tab object after `skills`:

```tsx
{
  id: 'canvas-tracker',
  label: t('nav.canvasTracker'),
  path: '/canvas-tracker',
  icon: (
    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={1.8}
        d="M9 5h6m-7 4h8m-8 4h5m-8 8h14a2 2 0 002-2V7.5L16.5 3H5a2 2 0 00-2 2v14a2 2 0 002 2z"
      />
    </svg>
  ),
},
```

Modify `isActive`:

```tsx
if (path === '/canvas-tracker') return location.pathname.startsWith('/canvas-tracker');
```

Add i18n keys:

```ts
'nav.canvasTracker': 'Canvas',
```

For Indonesian:

```ts
'nav.canvasTracker': 'Canvas',
```

For Chinese:

```ts
'nav.canvasTracker': 'Canvas',
```

- [ ] **Step 5: Run page tests and typecheck**

Run:

```bash
pnpm --filter openhuman-app test -- src/pages/__tests__/CanvasTracker.test.tsx
pnpm --filter openhuman-app compile
```

Expected: PASS.

- [ ] **Step 6: Commit Task 6**

```bash
git add app/src/pages/CanvasTracker.tsx app/src/pages/__tests__/CanvasTracker.test.tsx app/src/AppRoutes.tsx app/src/components/BottomTabBar.tsx app/src/lib/i18n/en.ts app/src/lib/i18n/id.ts app/src/lib/i18n/zh-CN.ts
git commit -m "feat(canvas): add tracker page"
```

## Task 7: End-To-End Verification And Safety Audit

**Files:**
- Modify only files touched by Tasks 1-6 if tests reveal defects.

- [ ] **Step 1: Run focused Rust checks**

Run:

```bash
cargo test --manifest-path Cargo.toml canvas_tracker -- --nocapture
cargo check --manifest-path Cargo.toml
```

Expected: PASS.

- [ ] **Step 2: Run focused frontend checks**

Run:

```bash
pnpm --filter openhuman-app test -- src/lib/canvasTracker/canvasTrackerApi.test.ts src/pages/__tests__/CanvasTracker.test.tsx
pnpm --filter openhuman-app compile
```

Expected: PASS.

- [ ] **Step 3: Audit safety gates**

Run:

```bash
rg -n "canvas_tracker|CanvasEndpoint|canvas_tracker_update|POST|PUT|PATCH|DELETE|submissions|submit" src/openhuman/canvas_tracker src/core/all.rs app/src/lib/canvasTracker app/src/pages/CanvasTracker.tsx
```

Expected:
- No `reqwest::Client::post`, `.put`, `.patch`, or `.delete` in `src/openhuman/canvas_tracker`.
- No Canvas submissions endpoint in `CanvasRequestPolicy`.
- The frontend status selector calls only `openhuman.canvas_tracker_update_local_status`.
- The token string appears only in settings update input paths and is never rendered.

- [ ] **Step 4: Run formatting**

Run:

```bash
cargo fmt --manifest-path Cargo.toml --all
pnpm --filter openhuman-app format:check
```

Expected: PASS. If Prettier reports changed formatting, run `pnpm --filter openhuman-app format`, inspect the diff, and commit formatting with the implementation.

- [ ] **Step 5: Commit final fixes**

```bash
git status --short
git add src/openhuman/canvas_tracker src/openhuman/mod.rs src/core/all.rs app/src/lib/canvasTracker app/src/pages/CanvasTracker.tsx app/src/pages/__tests__/CanvasTracker.test.tsx app/src/AppRoutes.tsx app/src/components/BottomTabBar.tsx app/src/lib/i18n/en.ts app/src/lib/i18n/id.ts app/src/lib/i18n/zh-CN.ts
git commit -m "test(canvas): verify tracker safety gates"
```

## Self-Review

- Spec coverage: The plan covers course allowlist, read-only Canvas requests, local token storage, local task status, due-date uncertainty, deterministic summaries, urgency, recommended start dates, reminder recommendations, UI, tests, and safety audit.
- Scope check: LINE tracking, Canvas writes, OS notifications, grade tracking, and LLM summaries are excluded from implementation.
- Type consistency: Rust and TypeScript use the same snake_case wire names for `LocalStatus`, `UrgencyLevel`, `CanvasTask`, `CanvasTrackerSettings`, `ReminderRecommendation`, and `SyncSummary`.
- Test strategy: Pure rules are unit-tested first, storage is isolated with temp SQLite, HTTP policy is tested without live Canvas, frontend tests mock the hook and RPC wrapper, and final verification searches for disallowed Canvas write paths.
