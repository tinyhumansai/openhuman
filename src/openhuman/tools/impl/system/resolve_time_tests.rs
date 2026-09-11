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
