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
        Some("101"),
        "361100-Secrets of the Soil-Lec.001 | 801[3/68] Extra API suffix",
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

    let prefix_matchers = vec![CourseMatcher {
        canvas_id: None,
        name: "Course".to_string(),
    }];
    assert!(!course_matches_allowlist(
        Some("404"),
        "Coursework",
        &prefix_matchers
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
            &task_due_at(Some("2026-05-21T06:00:00Z"), LocalStatus::NotStarted),
            now
        ),
        UrgencyLevel::Medium
    );
    assert_eq!(
        classify_urgency(
            &task_due_at(Some("2026-05-24T06:00:00Z"), LocalStatus::NotStarted),
            now
        ),
        UrgencyLevel::Low
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
fn recommended_start_at_clamps_stale_computed_start_to_now() {
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();
    let mut task = task_due_at(Some("2026-05-19T07:00:00Z"), LocalStatus::NotStarted);
    task.instructions_summary = "Upload a project file.".to_string();

    assert_eq!(recommended_start_at(&task, now), Some(now.to_rfc3339()));
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

#[test]
fn reminder_plan_clamps_stale_due_24h_to_now() {
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();
    let task = task_due_at(Some("2026-05-16T12:00:00Z"), LocalStatus::InProgress);
    let reminders = reminder_plan(&task, now);
    let due_24h = reminders
        .iter()
        .find(|r| r.kind == "due_24h")
        .expect("due_24h reminder exists");

    assert_eq!(due_24h.at.as_deref(), Some(now.to_rfc3339().as_str()));
}

#[test]
fn due_morning_uses_bangkok_morning_before_due_time() {
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();
    let task = task_due_at(Some("2026-05-17T02:00:00Z"), LocalStatus::InProgress);
    let reminders = reminder_plan(&task, now);
    let due_morning = reminders
        .iter()
        .find(|r| r.kind == "due_morning")
        .expect("due_morning reminder exists");

    assert_eq!(due_morning.at.as_deref(), Some("2026-05-17T01:00:00+00:00"));
    assert_ne!(due_morning.at.as_deref(), Some("2026-05-17T08:00:00+00:00"));
}

#[test]
fn due_morning_falls_back_before_early_bangkok_due_time() {
    let now = Utc.with_ymd_and_hms(2026, 5, 16, 6, 0, 0).unwrap();
    let task = task_due_at(Some("2026-05-16T23:30:00Z"), LocalStatus::InProgress);
    let reminders = reminder_plan(&task, now);
    let due_morning = reminders
        .iter()
        .find(|r| r.kind == "due_morning")
        .expect("due_morning reminder exists");

    assert_eq!(due_morning.at.as_deref(), Some("2026-05-16T22:30:00+00:00"));
}
