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
    // Express `from` as local midnight so the expected next-09:00 is always on the
    // same calendar day, regardless of the host timezone.  A UTC-fixed `from` would
    // land at different local times on different machines (e.g. already 10:00 local
    // on a UTC+10 host), making the expected date machine-dependent.
    let from_local = chrono::Local
        .with_ymd_and_hms(2026, 2, 16, 0, 0, 0)
        .unwrap();
    let from = from_local.with_timezone(&Utc);
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
        active_hours: Some(ActiveHours {
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
        active_hours: Some(ActiveHours {
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
fn next_run_respects_active_hours_in_schedule_timezone() {
    let schedule = Schedule::Cron {
        expr: "0 * * * *".into(),
        tz: Some("America/Los_Angeles".into()),
        active_hours: Some(ActiveHours {
            start: "09:00".into(),
            end: "10:00".into(),
        }),
    };

    let from = Utc.with_ymd_and_hms(2026, 2, 16, 15, 30, 0).unwrap();
    let next = next_run_for_schedule(&schedule, from).unwrap();

    assert_eq!(next, Utc.with_ymd_and_hms(2026, 2, 16, 17, 0, 0).unwrap());
}

#[test]
fn validate_schedule_rejects_invalid_active_hours() {
    let now = Utc::now();
    let schedule = Schedule::Cron {
        expr: "* * * * *".into(),
        tz: None,
        active_hours: Some(ActiveHours {
            start: "invalid".into(),
            end: "09:00".into(),
        }),
    };
    assert!(validate_schedule(&schedule, now).is_err());
}

#[test]
fn validate_schedule_rejects_invalid_active_hours_end() {
    let now = Utc::now();
    let schedule = Schedule::Cron {
        expr: "* * * * *".into(),
        tz: Some("UTC".into()),
        active_hours: Some(ActiveHours {
            start: "09:00".into(),
            end: "24:00".into(),
        }),
    };
    let err = validate_schedule(&schedule, now).unwrap_err();
    assert!(err.to_string().contains("active_hours.end"));
}

// ── runs_closer_than / validate_agent_schedule (#6158) ──────────

fn utc_cron(expr: &str) -> Schedule {
    Schedule::Cron {
        expr: expr.into(),
        tz: Some("UTC".into()),
        active_hours: None,
    }
}

#[test]
fn runs_closer_than_reports_the_shortest_gap_of_an_irregular_expression() {
    // From :03 the pair that follows is :30 → 1:01 — 31 minutes apart, which
    // is what a "the pair after now" check would have judged the schedule by.
    // The :01 → :02 pair one minute apart is what counts.
    let from = Utc.with_ymd_and_hms(2026, 3, 2, 10, 3, 0).unwrap();
    let hit = runs_closer_than(&utc_cron("1,2,30 * * * *"), from, MIN_AGENT_JOB_INTERVAL)
        .expect("the one-minute gap must be found");
    assert_eq!(
        hit,
        TooFrequent::ConsecutiveRuns {
            first: Utc.with_ymd_and_hms(2026, 3, 2, 11, 1, 0).unwrap(),
            second: Utc.with_ymd_and_hms(2026, 3, 2, 11, 2, 0).unwrap(),
        }
    );
    assert_eq!(hit.gap(), ChronoDuration::minutes(1));
}

#[test]
fn runs_closer_than_does_not_depend_on_the_instant_it_starts_from() {
    for minute in [0, 1, 2, 3, 15, 29, 30, 31, 59] {
        let from = Utc.with_ymd_and_hms(2026, 3, 2, 10, minute, 0).unwrap();
        for expr in ["1,2,30 * * * *", "0,1 * * * *", "0,3 9 * * *"] {
            assert!(
                runs_closer_than(&utc_cron(expr), from, MIN_AGENT_JOB_INTERVAL).is_some(),
                "{expr} from :{minute:02}"
            );
        }
        for expr in ["*/10 * * * *", "0 * * * *", "0,30 9 * * *"] {
            assert!(
                runs_closer_than(&utc_cron(expr), from, MIN_AGENT_JOB_INTERVAL).is_none(),
                "{expr} from :{minute:02}"
            );
        }
    }
}

/// The closest pair is reported, not the first one under the floor: `1,3,4,30`
/// has :01 → :03 (2 min) before :03 → :04 (1 min), and the message must name
/// the latter. Ties keep the earliest pair.
#[test]
fn runs_closer_than_reports_the_closest_pair_not_the_first_one_under_the_floor() {
    let from = Utc.with_ymd_and_hms(2026, 3, 2, 10, 0, 0).unwrap();
    let hit =
        runs_closer_than(&utc_cron("1,3,4,30 * * * *"), from, MIN_AGENT_JOB_INTERVAL).unwrap();
    assert_eq!(
        hit,
        TooFrequent::ConsecutiveRuns {
            first: Utc.with_ymd_and_hms(2026, 3, 2, 10, 3, 0).unwrap(),
            second: Utc.with_ymd_and_hms(2026, 3, 2, 10, 4, 0).unwrap(),
        }
    );
    // All gaps equal: the earliest pair wins.
    let hit = runs_closer_than(&utc_cron("*/2 * * * *"), from, MIN_AGENT_JOB_INTERVAL).unwrap();
    assert_eq!(
        hit,
        TooFrequent::ConsecutiveRuns {
            first: Utc.with_ymd_and_hms(2026, 3, 2, 10, 2, 0).unwrap(),
            second: Utc.with_ymd_and_hms(2026, 3, 2, 10, 4, 0).unwrap(),
        }
    );
}

/// `*/7` is 0,7,…,56: the :56 → :00 step is four minutes, and it counts.
#[test]
fn runs_closer_than_counts_the_wrap_around_gap() {
    let from = Utc.with_ymd_and_hms(2026, 3, 2, 10, 0, 0).unwrap();
    let hit = runs_closer_than(&utc_cron("*/7 * * * *"), from, MIN_AGENT_JOB_INTERVAL).unwrap();
    assert_eq!(hit.gap(), ChronoDuration::minutes(4));
    assert_eq!(
        hit.to_string(),
        "fires at 2026-03-02 10:56:00 UTC and again at 2026-03-02 11:00:00 UTC, 4 minutes apart"
    );
}

#[test]
fn runs_closer_than_is_quiet_at_and_above_the_threshold() {
    let from = Utc.with_ymd_and_hms(2026, 3, 2, 10, 0, 0).unwrap();
    for expr in [
        "*/5 * * * *",
        "*/6 * * * *",
        "0 * * * *",
        "0 9 * * *",
        "0 9 * * 1",
        "0 0 1 1 *",
        "0 */5 * * * *",
    ] {
        assert!(
            runs_closer_than(&utc_cron(expr), from, MIN_AGENT_JOB_INTERVAL).is_none(),
            "{expr}"
        );
    }
    let every_five = Schedule::Every { every_ms: 300_000 };
    assert!(runs_closer_than(&every_five, from, MIN_AGENT_JOB_INTERVAL).is_none());
    let once = Schedule::At { at: from };
    assert!(runs_closer_than(&once, from, MIN_AGENT_JOB_INTERVAL).is_none());
}

#[test]
fn runs_closer_than_reports_a_fixed_interval() {
    let from = Utc::now();
    let hit = runs_closer_than(
        &Schedule::Every { every_ms: 90_000 },
        from,
        MIN_AGENT_JOB_INTERVAL,
    )
    .unwrap();
    assert_eq!(hit, TooFrequent::FixedInterval(ChronoDuration::seconds(90)));
    assert_eq!(hit.to_string(), "fires every 1 minute 30 seconds");
    let just_under = Schedule::Every { every_ms: 299_999 };
    assert!(runs_closer_than(&just_under, from, MIN_AGENT_JOB_INTERVAL).is_some());
}

#[test]
fn runs_closer_than_sees_seconds_level_expressions() {
    let from = Utc.with_ymd_and_hms(2026, 3, 2, 10, 0, 0).unwrap();
    let hit = runs_closer_than(&utc_cron("*/30 * * * * *"), from, MIN_AGENT_JOB_INTERVAL).unwrap();
    assert_eq!(hit.gap(), ChronoDuration::seconds(30));
}

#[test]
fn runs_closer_than_judges_the_effective_cadence_inside_the_active_window() {
    let from = Utc.with_ymd_and_hms(2026, 3, 2, 10, 0, 0).unwrap();
    let every_minute_during = |start: &str, end: &str| Schedule::Cron {
        expr: "* * * * *".into(),
        tz: Some("UTC".into()),
        active_hours: Some(ActiveHours {
            start: start.into(),
            end: end.into(),
        }),
    };
    // Every minute, but only during one minute of the day: effectively daily.
    let daily = every_minute_during("09:00", "09:00");
    assert!(runs_closer_than(&daily, from, MIN_AGENT_JOB_INTERVAL).is_none());
    // A two-minute window lets two adjacent runs through. Skipping ~1,438
    // out-of-window candidates per day exhausts the shared budget long before
    // the 1,000-run walk ends; the pair found before that is still reported.
    let hit = runs_closer_than(
        &every_minute_during("09:00", "09:01"),
        from,
        MIN_AGENT_JOB_INTERVAL,
    )
    .unwrap();
    assert_eq!(hit.gap(), ChronoDuration::minutes(1));
}

#[test]
fn runs_closer_than_treats_an_unreadable_expression_as_no_evidence() {
    let from = Utc::now();
    assert!(runs_closer_than(&utc_cron("not a cron"), from, MIN_AGENT_JOB_INTERVAL).is_none());
    let bad_tz = Schedule::Cron {
        expr: "* * * * *".into(),
        tz: Some("Mars/Olympus_Mons".into()),
        active_hours: None,
    };
    assert!(runs_closer_than(&bad_tz, from, MIN_AGENT_JOB_INTERVAL).is_none());
}

#[test]
fn validate_agent_schedule_rejects_tight_schedules_and_names_the_evidence() {
    let now = Utc.with_ymd_and_hms(2026, 3, 2, 10, 0, 0).unwrap();
    let err = validate_agent_schedule(&utc_cron("*/3 * * * *"), now)
        .unwrap_err()
        .to_string();
    assert_eq!(
        err,
        "Invalid schedule: agent jobs must run at least 5 minutes apart, but this schedule \
         fires at 2026-03-02 10:03:00 UTC and again at 2026-03-02 10:06:00 UTC, 3 minutes apart"
    );
    let err = validate_agent_schedule(&Schedule::Every { every_ms: 60_000 }, now)
        .unwrap_err()
        .to_string();
    assert_eq!(
        err,
        "Invalid schedule: agent jobs must run at least 5 minutes apart, but this schedule \
         fires every 1 minute"
    );
}

#[test]
fn validate_agent_schedule_accepts_the_floor_and_keeps_the_generic_checks() {
    let now = Utc::now();
    assert!(validate_agent_schedule(&utc_cron("*/5 * * * *"), now).is_ok());
    assert!(validate_agent_schedule(&Schedule::Every { every_ms: 300_000 }, now).is_ok());
    let soon = Schedule::At {
        at: now + ChronoDuration::minutes(1),
    };
    assert!(validate_agent_schedule(&soon, now).is_ok());

    // The generic validation runs first: a broken schedule is rejected as
    // broken, not as "too frequent".
    let err = validate_agent_schedule(&utc_cron("not a cron"), now)
        .unwrap_err()
        .to_string();
    assert!(err.contains("Invalid cron expression"), "{err}");
    let err = validate_agent_schedule(&Schedule::Every { every_ms: 0 }, now)
        .unwrap_err()
        .to_string();
    assert!(err.contains("every_ms must be > 0"), "{err}");
    let err = validate_agent_schedule(&Schedule::At { at: now }, now)
        .unwrap_err()
        .to_string();
    assert!(err.contains("'at' must be in the future"), "{err}");
}

#[test]
fn describe_gap_reads_naturally() {
    assert_eq!(describe_gap(ChronoDuration::seconds(1)), "1 second");
    assert_eq!(describe_gap(ChronoDuration::seconds(30)), "30 seconds");
    assert_eq!(describe_gap(ChronoDuration::minutes(1)), "1 minute");
    assert_eq!(describe_gap(ChronoDuration::minutes(5)), "5 minutes");
    assert_eq!(
        describe_gap(ChronoDuration::seconds(90)),
        "1 minute 30 seconds"
    );
}
