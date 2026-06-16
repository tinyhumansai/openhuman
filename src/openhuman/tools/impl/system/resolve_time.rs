//! Tool: resolve_time — convert a relative or absolute time expression into an
//! exact timestamp in every format a downstream tool might want.
//!
//! Motivation: LLMs are unreliable at Unix-epoch arithmetic. A real incident
//! had the integrations agent compute "24 hours ago" as `1752189120`
//! (2025-07-10) instead of the intended 2026-06-09 — ~10 months off — then
//! fetch Slack history ascending from that wrong floor and never reach the
//! latest messages. `current_time` only returns *now*, so the agent still has
//! to subtract by hand; this tool does the resolution deterministically and
//! returns the value ready to paste into a tool argument
//! (`oldest`/`latest`/`since`/`after`, cron times, …).
//!
//! Read-only, no side effects. Accepts:
//!   - `"now"`
//!   - past relative durations: `"24h ago"`, `"last 24 hours"`, `"-7d"`,
//!     `"30d"`, `"15m"`, `"2 weeks ago"` (units: s/m/h/d/w)
//!   - future relative durations (for scheduling): `"in 10 minutes"`,
//!     `"30m from now"`, `"+2h"`, `"next 7d"`
//!   - day anchors: `"today"`, `"yesterday"`, `"tomorrow"` (civil midnight in
//!     the resolved zone)
//!   - absolute: RFC-3339 (`"2026-06-09T19:12:00Z"`), bare date (`"2026-06-09"`),
//!     or `"YYYY-MM-DD HH:MM:SS"`
//!
//! Returns every common representation so the caller can pick the one the
//! target tool's schema wants:
//!   - `unix_s`     — Unix seconds (integer)
//!   - `unix_ms`    — Unix milliseconds (integer)
//!   - `slack_ts`   — Slack `conversations.history` style `"<secs>.000000"`
//!   - `rfc3339`    — `"2026-06-09T19:12:00+00:00"`
//!   - `value`      — the representation named by the optional `format` arg
//!                    (defaults to `unix_s`), as a string, for copy-paste.

use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, SecondsFormat, Utc};
use chrono_tz::Tz;
use serde_json::json;

pub struct ResolveTimeTool;

impl ResolveTimeTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ResolveTimeTool {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a relative-duration expression into a **signed** [`Duration`] offset
/// from now: negative = the past, positive = the future.
///
/// Direction comes from explicit markers — `"… ago"`, `"last "`, `"past "`, or
/// a leading `-` mean the past; `"in "`, `"next "`, `"… from now"`, or a
/// leading `+` mean the future. A bare duration (`"24h"`, `"7d"`) defaults to
/// the **past**, since the dominant caller is "recent / last N" history
/// lookups. Returns `None` if the input isn't a recognized relative duration.
///
/// Getting the sign right matters: `scheduler_agent` passes future phrasing
/// like `"in 10 minutes"`, which must resolve forward, not backward.
fn parse_relative_duration(raw: &str) -> Option<Duration> {
    let mut s = raw.trim().to_ascii_lowercase();
    let mut future = false;

    // Suffix direction markers.
    if let Some(rest) = s.strip_suffix(" ago") {
        s = rest.trim().to_string();
    } else if let Some(rest) = s.strip_suffix(" from now") {
        future = true;
        s = rest.trim().to_string();
    }
    // Prefix direction markers (first match wins).
    for (prefix, is_future) in [
        ("in ", true),
        ("next ", true),
        ("last ", false),
        ("past ", false),
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            future = is_future;
            s = rest.trim().to_string();
            break;
        }
    }
    if let Some(rest) = s.strip_prefix('+') {
        future = true;
        s = rest.trim().to_string();
    } else if let Some(rest) = s.strip_prefix('-') {
        // Leading '-' is the past — which is already the default — just strip it.
        s = rest.trim().to_string();
    }

    // Collapse internal whitespace between the number and the unit.
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");

    // Split into leading number + trailing unit (with or without a space).
    let split_at = s.find(|c: char| !c.is_ascii_digit())?;
    if split_at == 0 {
        return None; // no leading number
    }
    let (num_str, unit_str) = s.split_at(split_at);
    let n: i64 = num_str.trim().parse().ok()?;
    let unit = unit_str.trim();

    let secs_per = match unit {
        "s" | "sec" | "secs" | "second" | "seconds" => 1,
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3_600,
        "d" | "day" | "days" => 86_400,
        "w" | "wk" | "wks" | "week" | "weeks" => 604_800,
        _ => return None,
    };
    let magnitude = Duration::seconds(n.saturating_mul(secs_per));
    Some(if future { magnitude } else { -magnitude })
}

/// Resolve `expr` to an absolute UTC instant. `zone` interprets civil
/// inputs (`today`, `yesterday`, bare dates without an explicit offset).
fn resolve_expr(expr: &str, zone: ResolveZone) -> Result<DateTime<Utc>, String> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Err(
            "`expr` is required (e.g. \"24h ago\", \"2026-06-09T19:12:00Z\", \"now\").".into(),
        );
    }
    let lower = trimmed.to_ascii_lowercase();

    if lower == "now" {
        return Ok(Utc::now());
    }

    // Relative duration → signed offset from now (sign encodes past/future).
    if let Some(dur) = parse_relative_duration(trimmed) {
        return Ok(Utc::now() + dur);
    }

    // Day anchors: civil midnight in the resolved zone.
    if lower == "today" || lower == "yesterday" || lower == "tomorrow" {
        let offset_days = match lower.as_str() {
            "yesterday" => -1,
            "tomorrow" => 1,
            _ => 0,
        };
        let today_civil = zone.now_civil_date();
        let target = today_civil + Duration::days(offset_days);
        return zone.civil_midnight_to_utc(target);
    }

    // RFC-3339 / ISO-8601 with explicit offset (e.g. ...Z, +05:30).
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(dt.with_timezone(&Utc));
    }

    // "YYYY-MM-DD HH:MM:SS" or "YYYY-MM-DDTHH:MM:SS" (no offset) → resolve in zone.
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(trimmed, fmt) {
            return zone.naive_to_utc(naive);
        }
    }

    // Bare date "YYYY-MM-DD" → civil midnight in zone.
    if let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return zone.civil_midnight_to_utc(date);
    }

    Err(format!(
        "could not parse time expression {trimmed:?}. Accepted: \"now\", a relative \
         duration — past (\"24h ago\" / \"7d\" / \"2 weeks ago\") or future \
         (\"in 10 minutes\" / \"30m from now\") with units s/m/h/d/w, \
         \"today\" / \"yesterday\" / \"tomorrow\", an RFC-3339 timestamp like \
         \"2026-06-09T19:12:00Z\", a bare date \"2026-06-09\", or \
         \"YYYY-MM-DD HH:MM:SS\"."
    ))
}

/// Zone used to interpret civil (offset-less) inputs.
enum ResolveZone {
    /// Machine-local timezone (the default, matching `current_time`).
    Local,
    /// An explicit IANA zone supplied by the caller.
    Iana(Tz),
}

impl ResolveZone {
    fn now_civil_date(&self) -> NaiveDate {
        match self {
            ResolveZone::Local => Local::now().date_naive(),
            ResolveZone::Iana(tz) => Utc::now().with_timezone(tz).date_naive(),
        }
    }

    fn civil_midnight_to_utc(&self, date: NaiveDate) -> Result<DateTime<Utc>, String> {
        let naive = date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| "invalid civil midnight".to_string())?;
        self.naive_to_utc(naive)
    }

    /// Interpret a naive (offset-less) datetime in this zone and convert to UTC.
    fn naive_to_utc(&self, naive: NaiveDateTime) -> Result<DateTime<Utc>, String> {
        use chrono::TimeZone;
        match self {
            ResolveZone::Local => Local
                .from_local_datetime(&naive)
                .single()
                .map(|dt| dt.with_timezone(&Utc))
                .ok_or_else(|| format!("ambiguous or invalid local time {naive} (DST boundary?)")),
            ResolveZone::Iana(tz) => tz
                .from_local_datetime(&naive)
                .single()
                .map(|dt| dt.with_timezone(&Utc))
                .ok_or_else(|| {
                    format!("ambiguous or invalid time {naive} in {tz:?} (DST boundary?)")
                }),
        }
    }
}

#[async_trait]
impl Tool for ResolveTimeTool {
    fn name(&self) -> &str {
        "resolve_time"
    }

    fn description(&self) -> &str {
        "Resolve a relative or absolute time expression into an exact timestamp. \
         ALWAYS use this to produce any date/time argument for another tool \
         (Slack/Gmail/Calendar `oldest`/`latest`/`since`/`after`, cron times, etc.) — \
         never hand-compute Unix/epoch seconds yourself; LLM epoch arithmetic is \
         unreliable. Pass `expr` as \"now\", a past relative duration (\"24h ago\", \
         \"7d\", \"2 weeks ago\"), a future one for scheduling (\"in 10 minutes\", \
         \"30m from now\"), \"today\"/\"yesterday\"/\"tomorrow\", an RFC-3339 timestamp \
         (\"2026-06-09T19:12:00Z\"), or a date (\"2026-06-09\"). The result gives \
         `unix_s`, `unix_ms`, `slack_ts`, and `rfc3339` — copy the one the target \
         tool's schema wants. For \"recent / last N\" lookups prefer newest-first \
         (omit `oldest`) so a wrong floor can't bury the latest data."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "expr": {
                    "type": "string",
                    "description": "Time expression: \"now\", a past duration \
                                    (\"24h ago\", \"7d\", \"2 weeks ago\"), a future \
                                    duration (\"in 10 minutes\", \"30m from now\"), \
                                    \"today\"/\"yesterday\"/\"tomorrow\", \
                                    \"2026-06-09T19:12:00Z\", \"2026-06-09\", or \
                                    \"YYYY-MM-DD HH:MM:SS\"."
                },
                "format": {
                    "type": "string",
                    "enum": ["unix_s", "unix_ms", "slack_ts", "rfc3339"],
                    "description": "Which representation to put in the top-level `value` \
                                    field (all representations are always returned too). \
                                    Defaults to unix_s."
                },
                "timezone": {
                    "type": "string",
                    "description": "Optional IANA timezone (e.g. 'Asia/Kolkata') used to \
                                    interpret offset-less inputs like 'today' or \
                                    '2026-06-09'. Defaults to the machine's local zone. \
                                    Ignored for inputs that already carry an offset."
                }
            },
            "required": ["expr"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_options(args, ToolCallOptions::default())
            .await
    }

    async fn execute_with_options(
        &self,
        args: serde_json::Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        tracing::debug!(args = %args, "[resolve_time] execute start");

        let expr = match args.get("expr").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return Ok(ToolResult::error(
                    "resolve_time: `expr` is required (e.g. \"24h ago\", \
                     \"2026-06-09T19:12:00Z\", \"now\").",
                ));
            }
        };

        // Resolve the interpretation zone for civil inputs.
        let zone = match args.get("timezone").and_then(|v| v.as_str()) {
            Some(tz_name) if !tz_name.trim().is_empty() => match tz_name.trim().parse::<Tz>() {
                Ok(tz) => ResolveZone::Iana(tz),
                Err(_) => {
                    return Ok(ToolResult::error(format!(
                        "resolve_time: unknown IANA timezone '{}' — use names like \
                         'America/Los_Angeles'.",
                        tz_name.trim()
                    )));
                }
            },
            _ => ResolveZone::Local,
        };

        let dt = match resolve_expr(expr, zone) {
            Ok(dt) => dt,
            Err(e) => {
                tracing::debug!(expr = expr, error = %e, "[resolve_time] parse failed");
                return Ok(ToolResult::error(format!("resolve_time: {e}")));
            }
        };

        let unix_s = dt.timestamp();
        let unix_ms = dt.timestamp_millis();
        let slack_ts = format!("{unix_s}.000000");
        let rfc3339 = dt.to_rfc3339_opts(SecondsFormat::Secs, true);

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("unix_s");
        let value = match format {
            "unix_ms" => unix_ms.to_string(),
            "slack_ts" => slack_ts.clone(),
            "rfc3339" => rfc3339.clone(),
            _ => unix_s.to_string(),
        };

        let payload = json!({
            "interpreted": expr,
            "value": value,
            "unix_s": unix_s,
            "unix_ms": unix_ms,
            "slack_ts": slack_ts,
            "rfc3339": rfc3339,
        });

        tracing::debug!("[resolve_time] resolved {expr:?} -> {rfc3339} (unix_s={unix_s})");
        let mut result = ToolResult::success(serde_json::to_string_pretty(&payload)?);
        if options.prefer_markdown {
            result.markdown_formatted = Some(format!(
                "- **interpreted**: {expr}\n- **value** ({format}): {value}\n- **unix_s**: \
                 {unix_s}\n- **unix_ms**: {unix_ms}\n- **slack_ts**: {slack_ts}\n- **rfc3339**: \
                 {rfc3339}\n"
            ));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_permission() {
        let tool = ResolveTimeTool::new();
        assert_eq!(tool.name(), "resolve_time");
        assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    }

    #[test]
    fn schema_requires_expr() {
        let schema = ResolveTimeTool::new().parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"][0], "expr");
    }

    #[test]
    fn past_variants_resolve_to_a_negative_offset() {
        // Past phrasing (and bare durations, which default to the past) yield a
        // NEGATIVE offset so `now + dur` looks backward.
        for s in [
            "24h ago",
            "last 24 hours",
            "past 24 hours",
            "-24h",
            "24 hours ago",
            "24h",
        ] {
            let d = parse_relative_duration(s).unwrap_or_else(|| panic!("failed: {s}"));
            assert_eq!(d.num_seconds(), -86_400, "{s}");
        }
        assert_eq!(
            parse_relative_duration("7d").unwrap().num_seconds(),
            -604_800
        );
        assert_eq!(
            parse_relative_duration("2 weeks").unwrap().num_seconds(),
            -1_209_600
        );
        assert_eq!(parse_relative_duration("15m").unwrap().num_seconds(), -900);
        assert_eq!(
            parse_relative_duration("30 days").unwrap().num_seconds(),
            -2_592_000
        );
    }

    #[test]
    fn future_variants_resolve_to_a_positive_offset() {
        // Regression for the CodeRabbit catch: future phrasing must look
        // FORWARD (positive offset), not backward — scheduler_agent relies on
        // "in 10 minutes" / "30m from now".
        assert_eq!(
            parse_relative_duration("in 10 minutes")
                .unwrap()
                .num_seconds(),
            600
        );
        assert_eq!(
            parse_relative_duration("30m from now")
                .unwrap()
                .num_seconds(),
            1_800
        );
        assert_eq!(parse_relative_duration("+2h").unwrap().num_seconds(), 7_200);
        assert_eq!(
            parse_relative_duration("next 7d").unwrap().num_seconds(),
            604_800
        );
    }

    #[test]
    fn rejects_non_durations() {
        assert!(parse_relative_duration("now").is_none());
        assert!(parse_relative_duration("2026-06-09").is_none());
        assert!(parse_relative_duration("h").is_none());
        assert!(parse_relative_duration("24 lightyears").is_none());
    }

    #[test]
    fn resolves_rfc3339_to_exact_utc() {
        let dt = resolve_expr("2026-06-09T19:12:00Z", ResolveZone::Local).unwrap();
        // The exact epoch the real incident's agent miscomputed as 1752189120.
        assert_eq!(dt.timestamp(), 1_781_032_320);
    }

    #[test]
    fn relative_is_close_to_now_minus_offset() {
        let before = Utc::now().timestamp();
        let dt = resolve_expr("24h ago", ResolveZone::Local).unwrap();
        let after = Utc::now().timestamp();
        let expected_lo = before - 86_400 - 2;
        let expected_hi = after - 86_400 + 2;
        assert!(
            dt.timestamp() >= expected_lo && dt.timestamp() <= expected_hi,
            "got {}, expected ~[{expected_lo},{expected_hi}]",
            dt.timestamp()
        );
    }

    #[test]
    fn future_relative_resolves_forward() {
        // "in 10 minutes" must land ~600s in the FUTURE (the bug fix).
        let before = Utc::now().timestamp();
        let dt = resolve_expr("in 10 minutes", ResolveZone::Local).unwrap();
        let after = Utc::now().timestamp();
        assert!(
            dt.timestamp() >= before + 600 - 2 && dt.timestamp() <= after + 600 + 2,
            "got {}, expected ~now+600",
            dt.timestamp()
        );
    }

    #[test]
    fn tomorrow_is_after_today_after_yesterday() {
        let tz: Tz = "Asia/Kolkata".parse().unwrap();
        let y = resolve_expr("yesterday", ResolveZone::Iana(tz)).unwrap();
        let t = resolve_expr("today", ResolveZone::Iana(tz)).unwrap();
        let m = resolve_expr("tomorrow", ResolveZone::Iana(tz)).unwrap();
        assert!(y < t && t < m, "ordering broken: {y} {t} {m}");
        // Consecutive civil days are exactly 24h apart.
        assert_eq!((t - y).num_seconds(), 86_400);
        assert_eq!((m - t).num_seconds(), 86_400);
    }

    #[test]
    fn now_resolves() {
        let dt = resolve_expr("now", ResolveZone::Local).unwrap();
        assert!((dt.timestamp() - Utc::now().timestamp()).abs() <= 2);
    }

    #[test]
    fn bare_date_in_explicit_zone() {
        // 2026-06-09 00:00 in Asia/Kolkata (UTC+5:30) == 2026-06-08T18:30:00Z.
        let tz: Tz = "Asia/Kolkata".parse().unwrap();
        let dt = resolve_expr("2026-06-09", ResolveZone::Iana(tz)).unwrap();
        assert_eq!(
            dt.to_rfc3339_opts(SecondsFormat::Secs, true),
            "2026-06-08T18:30:00Z"
        );
    }

    #[test]
    fn unparseable_expr_errors() {
        assert!(resolve_expr("sometime next quarter", ResolveZone::Local).is_err());
        assert!(resolve_expr("", ResolveZone::Local).is_err());
    }

    #[tokio::test]
    async fn execute_returns_all_formats() {
        let result = ResolveTimeTool::new()
            .execute(json!({ "expr": "2026-06-09T19:12:00Z" }))
            .await
            .unwrap();
        assert!(!result.is_error);
        let payload: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
        assert_eq!(payload["unix_s"], 1_781_032_320_i64);
        assert_eq!(payload["unix_ms"], 1_781_032_320_000_i64);
        assert_eq!(payload["slack_ts"], "1781032320.000000");
        assert_eq!(payload["value"], "1781032320"); // default format = unix_s
    }

    #[tokio::test]
    async fn execute_format_selects_value() {
        let result = ResolveTimeTool::new()
            .execute(json!({ "expr": "2026-06-09T19:12:00Z", "format": "slack_ts" }))
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
        assert_eq!(payload["value"], "1781032320.000000");
    }

    #[tokio::test]
    async fn execute_missing_expr_errors() {
        let result = ResolveTimeTool::new().execute(json!({})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("`expr` is required"));
    }

    #[tokio::test]
    async fn execute_bad_timezone_errors() {
        let result = ResolveTimeTool::new()
            .execute(json!({ "expr": "today", "timezone": "Not/AZone" }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("unknown IANA timezone"));
    }
}
