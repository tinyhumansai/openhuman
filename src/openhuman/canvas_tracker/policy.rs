use chrono::{DateTime, Duration, Utc};
use chrono_tz::Asia::Bangkok;
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
        id_matches
            || normalized == matcher_name
            || course_name_has_matcher_prefix(&normalized, &matcher_name)
    })
}

fn course_name_has_matcher_prefix(course_name: &str, matcher_name: &str) -> bool {
    let Some(suffix) = course_name.strip_prefix(matcher_name) else {
        return false;
    };
    suffix
        .chars()
        .next()
        .is_some_and(|ch| ch.is_whitespace() || ch.is_ascii_punctuation())
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
            due - if complex {
                Duration::days(4)
            } else {
                Duration::days(2)
            }
        }
        UrgencyLevel::Unclear => return None,
    };
    Some(start.max(now).to_rfc3339())
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
            at: Some(clamp_reminder_time(due - Duration::days(3), now).to_rfc3339()),
            message: "Assignment is due in 3 days.".to_string(),
        },
        ReminderRecommendation {
            kind: "due_24h".to_string(),
            at: Some(clamp_reminder_time(due - Duration::hours(24), now).to_rfc3339()),
            message: "Assignment is due in 24 hours.".to_string(),
        },
        ReminderRecommendation {
            kind: "due_morning".to_string(),
            at: Some(due_morning_at(due, now).to_rfc3339()),
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

fn clamp_reminder_time(reminder_at: DateTime<Utc>, now: DateTime<Utc>) -> DateTime<Utc> {
    reminder_at.max(now)
}

fn due_morning_at(due: DateTime<Utc>, now: DateTime<Utc>) -> DateTime<Utc> {
    let due_local = due.with_timezone(&Bangkok);
    let local_morning = due_local
        .date_naive()
        .and_hms_opt(8, 0, 0)
        .expect("valid Bangkok morning")
        .and_local_timezone(Bangkok)
        .single()
        .expect("Bangkok local time is unambiguous")
        .with_timezone(&Utc);
    let pre_deadline = if local_morning < due {
        local_morning
    } else {
        due - Duration::hours(1)
    };
    clamp_reminder_time(pre_deadline, now)
}
