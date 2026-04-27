use crate::openhuman::cron::Schedule;
use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use cron::Schedule as CronExprSchedule;
use std::str::FromStr;

pub fn next_run_for_schedule(schedule: &Schedule, from: DateTime<Utc>) -> Result<DateTime<Utc>> {
    match schedule {
        Schedule::Cron {
            expr,
            tz,
            active_hours,
        } => {
            let normalized = normalize_expression(expr)?;
            let cron = CronExprSchedule::from_str(&normalized)
                .with_context(|| format!("Invalid cron expression: {expr}"))?;

            let mut current_from = from;
            for _ in 0..100_000 {
                let next_utc = if let Some(tz_name) = tz {
                    let timezone = chrono_tz::Tz::from_str(tz_name)
                        .with_context(|| format!("Invalid IANA timezone: {tz_name}"))?;
                    let localized_from = current_from.with_timezone(&timezone);
                    let next_local = cron.after(&localized_from).next().ok_or_else(|| {
                        anyhow::anyhow!("No future occurrence for expression: {expr}")
                    })?;
                    next_local.with_timezone(&Utc)
                } else {
                    // Standardize on device-local timezone when tz is None.
                    let localized_from = current_from.with_timezone(&chrono::Local);
                    let next_local = cron.after(&localized_from).next().ok_or_else(|| {
                        anyhow::anyhow!("No future occurrence for expression: {expr}")
                    })?;
                    next_local.with_timezone(&Utc)
                };

                if let Some(active) = active_hours {
                    if is_in_active_hours(next_utc, active, tz.as_deref())? {
                        return Ok(next_utc);
                    }
                    current_from = next_utc;
                } else {
                    return Ok(next_utc);
                }
            }
            anyhow::bail!("No future occurrence found within active hours after 100,000 attempts")
        }
        Schedule::At { at } => Ok(*at),
        Schedule::Every { every_ms } => {
            if *every_ms == 0 {
                anyhow::bail!("Invalid schedule: every_ms must be > 0");
            }
            let ms = i64::try_from(*every_ms).context("every_ms is too large")?;
            let delta = ChronoDuration::milliseconds(ms);
            from.checked_add_signed(delta)
                .ok_or_else(|| anyhow::anyhow!("every_ms overflowed DateTime"))
        }
    }
}

pub fn validate_schedule(schedule: &Schedule, now: DateTime<Utc>) -> Result<()> {
    match schedule {
        Schedule::Cron {
            expr, active_hours, ..
        } => {
            let _ = normalize_expression(expr)?;
            if let Some(active) = active_hours {
                let _ = chrono::NaiveTime::parse_from_str(&active.start, "%H:%M")
                    .with_context(|| format!("Invalid active_hours.start: {}", active.start))?;
                let _ = chrono::NaiveTime::parse_from_str(&active.end, "%H:%M")
                    .with_context(|| format!("Invalid active_hours.end: {}", active.end))?;
            }
            let _ = next_run_for_schedule(schedule, now)?;
            Ok(())
        }
        Schedule::At { at } => {
            if *at <= now {
                anyhow::bail!("Invalid schedule: 'at' must be in the future");
            }
            Ok(())
        }
        Schedule::Every { every_ms } => {
            if *every_ms == 0 {
                anyhow::bail!("Invalid schedule: every_ms must be > 0");
            }
            Ok(())
        }
    }
}

pub fn schedule_cron_expression(schedule: &Schedule) -> Option<String> {
    match schedule {
        Schedule::Cron { expr, .. } => Some(expr.clone()),
        _ => None,
    }
}

fn is_in_active_hours(
    time: DateTime<Utc>,
    active: &crate::openhuman::cron::ActiveHours,
    tz: Option<&str>,
) -> Result<bool> {
    use chrono::Timelike;

    let hm = if let Some(tz_name) = tz {
        let timezone = chrono_tz::Tz::from_str(tz_name)
            .with_context(|| format!("Invalid IANA timezone: {tz_name}"))?;
        let localized = time.with_timezone(&timezone);
        chrono::NaiveTime::from_hms_opt(localized.hour(), localized.minute(), 0).unwrap()
    } else {
        let localized = time.with_timezone(&chrono::Local);
        chrono::NaiveTime::from_hms_opt(localized.hour(), localized.minute(), 0).unwrap()
    };

    let start = chrono::NaiveTime::parse_from_str(&active.start, "%H:%M")
        .with_context(|| format!("Invalid active_hours.start: {}", active.start))?;
    let end = chrono::NaiveTime::parse_from_str(&active.end, "%H:%M")
        .with_context(|| format!("Invalid active_hours.end: {}", active.end))?;

    if start <= end {
        Ok(hm >= start && hm <= end)
    } else {
        // Window spans midnight (e.g., 22:00 to 06:00)
        Ok(hm >= start || hm <= end)
    }
}

pub fn normalize_expression(expression: &str) -> Result<String> {
    let expression = expression.trim();
    let field_count = expression.split_whitespace().count();

    match field_count {
        // standard crontab syntax: minute hour day month weekday
        5 => Ok(format!("0 {expression}")),
        // crate-native syntax includes seconds (+ optional year)
        6 | 7 => Ok(expression.to_string()),
        _ => anyhow::bail!(
            "Invalid cron expression: {expression} (expected 5, 6, or 7 fields, got {field_count})"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn next_run_for_schedule_supports_every_and_at() {
        let now = Utc::now();
        let every = Schedule::Every { every_ms: 60_000 };
        let next = next_run_for_schedule(&every, now).unwrap();
        assert!(next > now);

        let at = now + ChronoDuration::minutes(10);
        let at_schedule = Schedule::At { at };
        let next_at = next_run_for_schedule(&at_schedule, now).unwrap();
        assert_eq!(next_at, at);
    }

    #[test]
    fn next_run_for_schedule_supports_timezone() {
        let from = Utc.with_ymd_and_hms(2026, 2, 16, 0, 0, 0).unwrap();
        let schedule = Schedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("America/Los_Angeles".into()),
            active_hours: None,
        };

        let next = next_run_for_schedule(&schedule, from).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 2, 16, 17, 0, 0).unwrap());
    }

    // ── normalize_expression ────────────────────────────────────────

    #[test]
    fn normalize_expression_accepts_standard_5_field_crontab() {
        // 5 fields → seconds column prepended so `cron` crate is happy.
        assert_eq!(normalize_expression("0 9 * * *").unwrap(), "0 0 9 * * *");
        assert_eq!(
            normalize_expression("*/5 * * * *").unwrap(),
            "0 */5 * * * *"
        );
    }

    #[test]
    fn normalize_expression_accepts_6_and_7_field_crate_native() {
        // 6 = second minute hour dom mon dow
        assert_eq!(normalize_expression("0 0 9 * * *").unwrap(), "0 0 9 * * *");
        // 7 adds year
        assert_eq!(
            normalize_expression("0 0 9 * * * 2027").unwrap(),
            "0 0 9 * * * 2027"
        );
    }

    #[test]
    fn normalize_expression_trims_whitespace() {
        assert_eq!(
            normalize_expression("   0 9 * * *   ").unwrap(),
            "0 0 9 * * *"
        );
    }

    #[test]
    fn normalize_expression_rejects_wrong_field_counts() {
        assert!(normalize_expression("").is_err());
        assert!(normalize_expression("* *").is_err());
        assert!(normalize_expression("* * *").is_err());
        assert!(normalize_expression("* * * *").is_err());
        assert!(normalize_expression("* * * * * * * *").is_err());
    }

    // ── next_run_for_schedule ───────────────────────────────────────

    #[test]
    fn next_run_cron_without_tz_uses_local_by_default() {
        // Since we changed default to local, we need to be careful in tests.
        // We'll use a fixed Local time and see if it works.
        let from = Utc.with_ymd_and_hms(2026, 2, 16, 0, 0, 0).unwrap();
        let schedule = Schedule::Cron {
            expr: "0 9 * * *".into(),
            tz: None,
            active_hours: None,
        };
        let next = next_run_for_schedule(&schedule, from).unwrap();

        let expected_local = chrono::Local
            .with_ymd_and_hms(2026, 2, 16, 9, 0, 0)
            .unwrap();
        assert_eq!(next, expected_local.with_timezone(&Utc));
    }

    #[test]
    fn next_run_rejects_invalid_cron_expression() {
        let schedule = Schedule::Cron {
            expr: "not a cron".into(),
            tz: None,
            active_hours: None,
        };
        let err = next_run_for_schedule(&schedule, Utc::now()).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("invalid"));
    }

    #[test]
    fn next_run_rejects_invalid_timezone() {
        let schedule = Schedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("Not/A_Real_Tz".into()),
            active_hours: None,
        };
        let err = next_run_for_schedule(&schedule, Utc::now()).unwrap_err();
        assert!(err
            .to_string()
            .to_lowercase()
            .contains("invalid iana timezone"));
    }

    #[test]
    fn next_run_every_zero_is_rejected() {
        let schedule = Schedule::Every { every_ms: 0 };
        let err = next_run_for_schedule(&schedule, Utc::now()).unwrap_err();
        assert!(err.to_string().contains("every_ms must be > 0"));
    }

    #[test]
    fn next_run_at_returns_the_exact_time() {
        let at = Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap();
        let schedule = Schedule::At { at };
        let next = next_run_for_schedule(&schedule, Utc::now()).unwrap();
        assert_eq!(next, at);
    }

    // ── validate_schedule ───────────────────────────────────────────

    #[test]
    fn validate_schedule_rejects_past_at_time() {
        let now = Utc::now();
        let past = now - ChronoDuration::minutes(5);
        let schedule = Schedule::At { at: past };
        let err = validate_schedule(&schedule, now).unwrap_err();
        assert!(err.to_string().contains("'at' must be in the future"));
    }

    #[test]
    fn validate_schedule_accepts_future_at_time() {
        let now = Utc::now();
        let future = now + ChronoDuration::minutes(5);
        let schedule = Schedule::At { at: future };
        assert!(validate_schedule(&schedule, now).is_ok());
    }

    #[test]
    fn validate_schedule_rejects_every_zero() {
        let schedule = Schedule::Every { every_ms: 0 };
        assert!(validate_schedule(&schedule, Utc::now()).is_err());
    }

    #[test]
    fn validate_schedule_accepts_valid_cron() {
        let now = Utc::now();
        let schedule = Schedule::Cron {
            expr: "*/5 * * * *".into(),
            tz: None,
            active_hours: None,
        };
        assert!(validate_schedule(&schedule, now).is_ok());
    }

    #[test]
    fn validate_schedule_rejects_garbage_cron_expression() {
        let schedule = Schedule::Cron {
            expr: "not a cron".into(),
            tz: None,
            active_hours: None,
        };
        assert!(validate_schedule(&schedule, Utc::now()).is_err());
    }

    // ── schedule_cron_expression ────────────────────────────────────

    #[test]
    fn schedule_cron_expression_returns_expr_for_cron_variant() {
        let s = Schedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("UTC".into()),
            active_hours: None,
        };
        assert_eq!(schedule_cron_expression(&s).as_deref(), Some("0 9 * * *"));
    }

    #[test]
    fn schedule_cron_expression_returns_none_for_non_cron_variants() {
        assert!(schedule_cron_expression(&Schedule::Every { every_ms: 1000 }).is_none());
        assert!(schedule_cron_expression(&Schedule::At { at: Utc::now() }).is_none());
    }

    #[test]
    fn next_run_respects_active_hours() {
        // Schedule: every minute
        // Active hours: 09:00 - 09:05
        let schedule = Schedule::Cron {
            expr: "* * * * *".into(),
            tz: Some("UTC".into()),
            active_hours: Some(crate::openhuman::cron::ActiveHours {
                start: "09:00".into(),
                end: "09:05".into(),
            }),
        };

        // If it's 08:00, next run should be 09:00
        let from = Utc.with_ymd_and_hms(2026, 2, 16, 8, 0, 0).unwrap();
        let next = next_run_for_schedule(&schedule, from).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 2, 16, 9, 0, 0).unwrap());

        // If it's 09:02, next run should be 09:03
        let from = Utc.with_ymd_and_hms(2026, 2, 16, 9, 2, 0).unwrap();
        let next = next_run_for_schedule(&schedule, from).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 2, 16, 9, 3, 0).unwrap());

        // If it's 09:05, next run should be 09:00 NEXT DAY
        let from = Utc.with_ymd_and_hms(2026, 2, 16, 9, 5, 0).unwrap();
        let next = next_run_for_schedule(&schedule, from).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 2, 17, 9, 0, 0).unwrap());
    }

    #[test]
    fn next_run_respects_active_hours_spanning_midnight() {
        // Active hours: 22:00 - 02:00
        let schedule = Schedule::Cron {
            expr: "0 * * * *".into(), // every hour
            tz: Some("UTC".into()),
            active_hours: Some(crate::openhuman::cron::ActiveHours {
                start: "22:00".into(),
                end: "02:00".into(),
            }),
        };

        // 20:00 -> 22:00
        let from = Utc.with_ymd_and_hms(2026, 2, 16, 20, 0, 0).unwrap();
        let next = next_run_for_schedule(&schedule, from).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 2, 16, 22, 0, 0).unwrap());

        // 23:00 -> 00:00
        let from = Utc.with_ymd_and_hms(2026, 2, 16, 23, 0, 0).unwrap();
        let next = next_run_for_schedule(&schedule, from).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 2, 17, 0, 0, 0).unwrap());

        // 01:00 -> 02:00
        let from = Utc.with_ymd_and_hms(2026, 2, 17, 1, 0, 0).unwrap();
        let next = next_run_for_schedule(&schedule, from).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 2, 17, 2, 0, 0).unwrap());

        // 03:00 -> 22:00 SAME DAY (since it's early morning)
        let from = Utc.with_ymd_and_hms(2026, 2, 17, 3, 0, 0).unwrap();
        let next = next_run_for_schedule(&schedule, from).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 2, 17, 22, 0, 0).unwrap());
    }

    #[test]
    fn validate_schedule_rejects_invalid_active_hours() {
        let now = Utc::now();
        let schedule = Schedule::Cron {
            expr: "* * * * *".into(),
            tz: None,
            active_hours: Some(crate::openhuman::cron::ActiveHours {
                start: "invalid".into(),
                end: "09:00".into(),
            }),
        };
        assert!(validate_schedule(&schedule, now).is_err());
    }
}
