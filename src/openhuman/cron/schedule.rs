use crate::openhuman::cron::{ActiveHours, Schedule};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, NaiveTime, Timelike, Utc};
use cron::Schedule as CronExprSchedule;
use std::fmt;
use std::str::FromStr;

/// The closest together two runs of an *agent* job may be scheduled. Every
/// run is a full inference turn, so a tighter cadence is almost always a
/// misconfiguration that bills accordingly. Shell and flow jobs are not
/// subject to it. Enforced by [`validate_agent_schedule`] when an agent job is
/// created or its schedule changed; the scheduler warns about rows that
/// predate the rule.
pub const MIN_AGENT_JOB_INTERVAL: ChronoDuration = ChronoDuration::minutes(5);

/// Upper bound on cron candidates walked while looking for an occurrence that
/// falls inside `active_hours`. `next_run_for_schedule` gets a fresh budget
/// per call; a `runs_closer_than` scan shares one across all of its steps.
const ACTIVE_WINDOW_CANDIDATE_LIMIT: usize = 100_000;

/// How many consecutive occurrences [`runs_closer_than`] walks before it
/// concludes a cron schedule keeps its distance. Every gap an hour- or
/// day-periodic expression can produce shows up well inside this many runs
/// (a schedule that respects a 5-minute floor fires at most 288 times a day),
/// so the verdict does not depend on the instant the scan starts from.
const RUN_GAP_SCAN_OCCURRENCES: usize = 1_000;

pub fn next_run_for_schedule(schedule: &Schedule, from: DateTime<Utc>) -> Result<DateTime<Utc>> {
    match schedule {
        Schedule::Cron {
            expr,
            tz,
            active_hours,
        } => {
            let plan = CronPlan::parse(expr, tz.as_deref(), active_hours.as_ref())?;
            let mut budget = ACTIVE_WINDOW_CANDIDATE_LIMIT;
            plan.next_after(from, &mut budget)
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
            expr,
            tz,
            active_hours,
        } => {
            let _ = normalize_expression(expr)?;
            if let Some(active) = active_hours {
                let _ = ActiveWindow::parse(active)?;
            }
            let _ = ScheduleTimeZone::parse(tz.as_deref())?;
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

/// [`validate_schedule`] plus the agent-only floor: an agent job may not run
/// closer together than [`MIN_AGENT_JOB_INTERVAL`]. The error names the two
/// runs (or the fixed interval) that break the rule, so the caller — an agent
/// using `cron_add`, or the settings form — can say exactly what to change.
pub fn validate_agent_schedule(schedule: &Schedule, now: DateTime<Utc>) -> Result<()> {
    validate_schedule(schedule, now)?;
    if let Some(too_frequent) = runs_closer_than(schedule, now, MIN_AGENT_JOB_INTERVAL) {
        anyhow::bail!(
            "Invalid schedule: agent jobs must run at least {} apart, but this schedule {too_frequent}",
            describe_gap(MIN_AGENT_JOB_INTERVAL)
        );
    }
    Ok(())
}

/// Why a schedule runs more often than a threshold allows. Carries the
/// evidence, not only the verdict, so a log line or an error can quote it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooFrequent {
    /// A fixed `every_ms` interval shorter than the threshold.
    FixedInterval(ChronoDuration),
    /// Two consecutive cron occurrences closer together than the threshold.
    ConsecutiveRuns {
        first: DateTime<Utc>,
        second: DateTime<Utc>,
    },
}

impl TooFrequent {
    /// The offending gap.
    pub fn gap(&self) -> ChronoDuration {
        match self {
            Self::FixedInterval(gap) => *gap,
            Self::ConsecutiveRuns { first, second } => *second - *first,
        }
    }
}

impl fmt::Display for TooFrequent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FixedInterval(gap) => write!(f, "fires every {}", describe_gap(*gap)),
            Self::ConsecutiveRuns { first, second } => write!(
                f,
                "fires at {} and again at {}, {} apart",
                first.format("%Y-%m-%d %H:%M:%S UTC"),
                second.format("%Y-%m-%d %H:%M:%S UTC"),
                describe_gap(*second - *first)
            ),
        }
    }
}

/// The two consecutive runs of `schedule` after `from` that are closest
/// together, if that gap is under `min_gap`.
///
/// Consecutive occurrences are walked in order and the smallest gap is kept,
/// so an irregular expression such as `1,3,4,30 * * * *` is judged by its
/// :03 → :04 pair — not by the first pair under the floor (:01 → :03) and not
/// by whichever pair happens to follow `from`. The wrap-around gap counts too:
/// `*/7 * * * *` fires at :56 and then at :00, four minutes apart, and is
/// reported as such. Ties keep the earliest pair.
///
/// The walk is bounded ([`RUN_GAP_SCAN_OCCURRENCES`] runs, one shared
/// [`ACTIVE_WINDOW_CANDIDATE_LIMIT`] budget), so a sparse or window-restricted
/// expression stays cheap; if the budget runs out mid-walk, the closest pair
/// seen so far is still reported. An expression that cannot be parsed, or that
/// has no second occurrence, is not evidence of anything and yields `None`;
/// [`validate_schedule`] is where a bad expression gets rejected.
pub fn runs_closer_than(
    schedule: &Schedule,
    from: DateTime<Utc>,
    min_gap: ChronoDuration,
) -> Option<TooFrequent> {
    match schedule {
        Schedule::At { .. } => None,
        Schedule::Every { every_ms } => {
            let gap = ChronoDuration::try_milliseconds(i64::try_from(*every_ms).ok()?)?;
            (gap < min_gap).then_some(TooFrequent::FixedInterval(gap))
        }
        Schedule::Cron {
            expr,
            tz,
            active_hours,
        } => {
            let plan = CronPlan::parse(expr, tz.as_deref(), active_hours.as_ref()).ok()?;
            let mut budget = ACTIVE_WINDOW_CANDIDATE_LIMIT;
            let mut previous = plan.next_after(from, &mut budget).ok()?;
            let mut closest: Option<TooFrequent> = None;
            for _ in 1..RUN_GAP_SCAN_OCCURRENCES {
                // Running out of budget (or of occurrences) ends the walk but
                // does not discard a pair already found.
                let Ok(next) = plan.next_after(previous, &mut budget) else {
                    break;
                };
                let gap = next - previous;
                if gap < min_gap && closest.is_none_or(|seen| gap < seen.gap()) {
                    closest = Some(TooFrequent::ConsecutiveRuns {
                        first: previous,
                        second: next,
                    });
                }
                previous = next;
            }
            closest
        }
    }
}

/// `4 minutes`, `30 seconds`, `1 minute 30 seconds` — whole seconds only.
fn describe_gap(gap: ChronoDuration) -> String {
    fn count(n: i64, unit: &str) -> String {
        if n == 1 {
            format!("{n} {unit}")
        } else {
            format!("{n} {unit}s")
        }
    }
    let seconds = gap.num_seconds();
    match (seconds / 60, seconds % 60) {
        (0, s) => count(s, "second"),
        (m, 0) => count(m, "minute"),
        (m, s) => format!("{} {}", count(m, "minute"), count(s, "second")),
    }
}

pub fn schedule_cron_expression(schedule: &Schedule) -> Option<String> {
    match schedule {
        Schedule::Cron { expr, .. } => Some(expr.clone()),
        _ => None,
    }
}

/// A [`Schedule::Cron`] parsed once, so walking many occurrences does not pay
/// for the expression, timezone and active-window parsing on every step.
struct CronPlan<'a> {
    expr: &'a str,
    cron: CronExprSchedule,
    timezone: ScheduleTimeZone,
    active_window: Option<ActiveWindow>,
}

impl<'a> CronPlan<'a> {
    fn parse(expr: &'a str, tz: Option<&str>, active_hours: Option<&ActiveHours>) -> Result<Self> {
        let normalized = normalize_expression(expr)?;
        let cron = CronExprSchedule::from_str(&normalized)
            .with_context(|| format!("Invalid cron expression: {expr}"))?;
        let timezone = ScheduleTimeZone::parse(tz)?;
        let active_window = active_hours.map(ActiveWindow::parse).transpose()?;
        Ok(Self {
            expr,
            cron,
            timezone,
            active_window,
        })
    }

    /// The first occurrence strictly after `from` that falls inside the active
    /// window, spending at most `budget` cron candidates to find it.
    fn next_after(&self, from: DateTime<Utc>, budget: &mut usize) -> Result<DateTime<Utc>> {
        let mut current_from = from;
        while *budget > 0 {
            *budget -= 1;
            let next_utc = self
                .timezone
                .next_after(&self.cron, current_from, self.expr)?;
            let Some(active) = &self.active_window else {
                return Ok(next_utc);
            };
            if active.contains(self.timezone.local_time_of_day(next_utc)) {
                return Ok(next_utc);
            }
            tracing::debug!(
                "[cron] next_run candidate {} outside active window {}–{}, advancing",
                next_utc,
                active.start,
                active.end
            );
            current_from = next_utc;
        }
        tracing::warn!(
            "[cron] no occurrence found within active_hours for expr={} after 100,000 candidates",
            self.expr
        );
        anyhow::bail!("No future occurrence found within active hours after 100,000 attempts")
    }
}

#[derive(Debug, Clone, Copy)]
enum ScheduleTimeZone {
    Local,
    Named(chrono_tz::Tz),
}

impl ScheduleTimeZone {
    fn parse(tz: Option<&str>) -> Result<Self> {
        match tz {
            Some(tz_name) => chrono_tz::Tz::from_str(tz_name)
                .map(Self::Named)
                .with_context(|| format!("Invalid IANA timezone: {tz_name}")),
            None => Ok(Self::Local),
        }
    }

    fn next_after(
        self,
        cron: &CronExprSchedule,
        from: DateTime<Utc>,
        expr: &str,
    ) -> Result<DateTime<Utc>> {
        match self {
            Self::Named(timezone) => {
                let localized_from = from.with_timezone(&timezone);
                let next_local = cron.after(&localized_from).next().ok_or_else(|| {
                    anyhow::anyhow!("No future occurrence for expression: {expr}")
                })?;
                Ok(next_local.with_timezone(&Utc))
            }
            Self::Local => {
                let localized_from = from.with_timezone(&chrono::Local);
                let next_local = cron.after(&localized_from).next().ok_or_else(|| {
                    anyhow::anyhow!("No future occurrence for expression: {expr}")
                })?;
                Ok(next_local.with_timezone(&Utc))
            }
        }
    }

    fn local_time_of_day(self, time: DateTime<Utc>) -> NaiveTime {
        match self {
            Self::Named(timezone) => {
                let localized = time.with_timezone(&timezone);
                NaiveTime::from_hms_opt(localized.hour(), localized.minute(), 0)
                    .expect("hour() and minute() from a valid DateTime are always in-range")
            }
            Self::Local => {
                let localized = time.with_timezone(&chrono::Local);
                NaiveTime::from_hms_opt(localized.hour(), localized.minute(), 0)
                    .expect("hour() and minute() from a valid DateTime are always in-range")
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ActiveWindow {
    start: NaiveTime,
    end: NaiveTime,
}

impl ActiveWindow {
    fn parse(active: &ActiveHours) -> Result<Self> {
        let start = NaiveTime::parse_from_str(&active.start, "%H:%M")
            .with_context(|| format!("Invalid active_hours.start: {}", active.start))?;
        let end = NaiveTime::parse_from_str(&active.end, "%H:%M")
            .with_context(|| format!("Invalid active_hours.end: {}", active.end))?;
        Ok(Self { start, end })
    }

    fn contains(self, time: NaiveTime) -> bool {
        if self.start <= self.end {
            time >= self.start && time <= self.end
        } else {
            // Window spans midnight (e.g. 22:00 to 06:00).
            time >= self.start || time <= self.end
        }
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
#[path = "schedule_tests.rs"]
mod tests;
