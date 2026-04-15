use crate::openhuman::cron::Schedule;
use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use cron::Schedule as CronExprSchedule;
use std::str::FromStr;

pub fn next_run_for_schedule(schedule: &Schedule, from: DateTime<Utc>) -> Result<DateTime<Utc>> {
    match schedule {
        Schedule::Cron { expr, tz } => {
            let normalized = normalize_expression(expr)?;
            let cron = CronExprSchedule::from_str(&normalized)
                .with_context(|| format!("Invalid cron expression: {expr}"))?;

            if let Some(tz_name) = tz {
                let timezone = chrono_tz::Tz::from_str(tz_name)
                    .with_context(|| format!("Invalid IANA timezone: {tz_name}"))?;
                let localized_from = from.with_timezone(&timezone);
                let next_local = cron.after(&localized_from).next().ok_or_else(|| {
                    anyhow::anyhow!("No future occurrence for expression: {expr}")
                })?;
                Ok(next_local.with_timezone(&Utc))
            } else {
                cron.after(&from)
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("No future occurrence for expression: {expr}"))
            }
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
        Schedule::Cron { expr, .. } => {
            let _ = normalize_expression(expr)?;
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
    fn next_run_cron_without_tz_uses_utc_by_default() {
        // 0 9 * * * at 2026-02-16 00:00Z → next UTC 9am same day.
        let from = Utc.with_ymd_and_hms(2026, 2, 16, 0, 0, 0).unwrap();
        let schedule = Schedule::Cron {
            expr: "0 9 * * *".into(),
            tz: None,
        };
        let next = next_run_for_schedule(&schedule, from).unwrap();
        assert_eq!(next, Utc.with_ymd_and_hms(2026, 2, 16, 9, 0, 0).unwrap());
    }

    #[test]
    fn next_run_rejects_invalid_cron_expression() {
        let schedule = Schedule::Cron {
            expr: "not a cron".into(),
            tz: None,
        };
        let err = next_run_for_schedule(&schedule, Utc::now()).unwrap_err();
        assert!(err.to_string().to_lowercase().contains("invalid"));
    }

    #[test]
    fn next_run_rejects_invalid_timezone() {
        let schedule = Schedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("Not/A_Real_Tz".into()),
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
        };
        assert!(validate_schedule(&schedule, now).is_ok());
    }

    #[test]
    fn validate_schedule_rejects_garbage_cron_expression() {
        let schedule = Schedule::Cron {
            expr: "not a cron".into(),
            tz: None,
        };
        assert!(validate_schedule(&schedule, Utc::now()).is_err());
    }

    // ── schedule_cron_expression ────────────────────────────────────

    #[test]
    fn schedule_cron_expression_returns_expr_for_cron_variant() {
        let s = Schedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("UTC".into()),
        };
        assert_eq!(schedule_cron_expression(&s).as_deref(), Some("0 9 * * *"));
    }

    #[test]
    fn schedule_cron_expression_returns_none_for_non_cron_variants() {
        assert!(schedule_cron_expression(&Schedule::Every { every_ms: 1000 }).is_none());
        assert!(schedule_cron_expression(&Schedule::At { at: Utc::now() }).is_none());
    }
}
