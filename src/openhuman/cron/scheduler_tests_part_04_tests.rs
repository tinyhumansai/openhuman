use super::*;

#[tokio::test]
async fn deliver_if_configured_empty_success_skips_chat_and_alert() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = proactive_job();
    // Successful but empty: nothing delivered anywhere.
    assert!(deliver_if_configured(&config, &job, "", true).await.is_ok());
    assert_eq!(cron_alerts(&config).await, 0);
}

#[test]
fn publish_cron_user_error_broadcasts_metadata_only_for_each_kind() {
    use crate::openhuman::web_chat::subscribe_web_channel_events;

    // Folded from two tests that both published `api_key_missing` to the
    // process-global bus and could false-pass off each other's broadcast under
    // parallel execution (CodeRabbit #4169). One subscription + serialized
    // publishes means each assertion can only be satisfied by THIS test's own
    // emission, so a regression in `publish_cron_user_error` actually fails.
    // The three tokens are exactly the `UserErrorKind` values classify.ts accepts.
    let mut rx = subscribe_web_channel_events();
    for kind in ["insufficient_credits", "budget_exceeded", "api_key_missing"] {
        publish_cron_user_error(kind);
        let ev = next_user_error(&mut rx, kind);
        // Broadcast to the "system" room every connected socket auto-joins.
        assert_eq!(ev.client_id, "system");
        // Stable kind token mirrors the frontend `UserErrorKind` discriminator.
        assert_eq!(ev.error_type.as_deref(), Some(kind));
        assert_eq!(ev.error_source.as_deref(), Some("cron"));
        // Metadata-only: a `user_error` NEVER carries the raw provider body
        // (CLAUDE.md) and is thread-less (no chat context).
        assert!(ev.message.is_none(), "user_error must not carry a raw body");
        assert!(ev.full_response.is_none());
        assert!(ev.thread_id.is_empty(), "cron user_error is thread-less");
        assert!(ev.request_id.is_empty());
    }
}

// TAURI-RUST-12K (end-to-end) — the predicate tests above key on hand-written
// wire strings; this test proves the REAL provider-generated error remains
// retryable when it only preserves reqwest's short send-error prefix. A cron
// agent job is routed to a keyless local provider (`AuthStyle::None`, LM Studio
// shape) whose server is offline: the chat workload skips the credential guard
// and attempts loopback HTTP. If the provider layer surfaces only
// `error sending request for url (...)`, without the refused errno/tcp-connect
// chain, cron must not treat it as a permanent local-provider halt because the
// same short prefix is also used for transient timeout/reset shapes.
#[tokio::test]
async fn cron_agent_job_short_loopback_send_error_stays_retryable() {
    use crate::openhuman::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    // Keyless local provider (`AuthStyle::None` → no credential requirement, so
    // the request proceeds to the HTTP connect). `chat_provider` routes the
    // chat workload to it; the slug resolves to LM Studio's default endpoint.
    config.cloud_providers = vec![CloudProviderCreds {
        id: "lmstudio-offline".into(),
        slug: "lmstudio".into(),
        label: "LM Studio".into(),
        endpoint: "http://127.0.0.1:1".into(),
        auth_style: AuthStyle::None,
        ..Default::default()
    }];
    config.default_model = Some("lmstudio:local-model".into());
    config.chat_provider = Some("lmstudio:local-model".into());
    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.prompt = Some("Say hello".into());

    let (success, output, raw) = run_agent_job(&config, &job).await;
    assert!(
        !success,
        "a cron agent job against an offline local provider must fail"
    );
    assert!(
        !is_local_provider_unreachable_failure(&JobType::Agent, raw.as_deref(), &output),
        "provider-generated short loopback send error must stay retryable without refused errno/tcp-connect evidence; got raw={raw:?}"
    );
}

// ── agent-job frequency check (#6158) ───────────────────────────
//
// The `Schedule::Cron` arm used to compare the next run after now with the
// next run after now plus one second — the same instant unless a run fell
// inside that second — so *every* cron-scheduled agent job warned, whatever
// its expression, and the three `// should not panic` tests could not tell.
// The verdict is now returned with its evidence and measured over
// consecutive runs, so each case below asserts it.

fn agent_job_on(schedule: Schedule) -> CronJob {
    let mut job = test_job("echo hi");
    job.job_type = JobType::Agent;
    job.schedule = schedule;
    job
}

fn cron_agent_job(expr: &str) -> CronJob {
    agent_job_on(Schedule::Cron {
        expr: expr.into(),
        tz: None,
        active_hours: None,
    })
}

#[test]
fn non_agent_and_one_shot_jobs_are_never_high_frequency() {
    // `test_job` runs every minute — but it is a shell job.
    let mut shell = test_job("echo hi");
    shell.job_type = JobType::Shell;
    assert!(agent_job_too_frequent(&shell).is_none());
    warn_if_high_frequency_agent_job(&shell);

    let once = agent_job_on(Schedule::At { at: Utc::now() });
    assert!(agent_job_too_frequent(&once).is_none());
    warn_if_high_frequency_agent_job(&once);
}

#[test]
fn every_ms_below_five_minutes_is_high_frequency() {
    let job = agent_job_on(Schedule::Every { every_ms: 60_000 });
    assert_eq!(
        agent_job_too_frequent(&job),
        Some(TooFrequent::FixedInterval(ChronoDuration::minutes(1)))
    );
    warn_if_high_frequency_agent_job(&job); // logs the evidence, must not panic
}

/// The regression: every ordinary schedule was reported as too frequent.
#[test]
fn an_ordinary_cron_agent_job_is_not_high_frequency() {
    for expr in [
        "*/10 * * * *",
        "0 * * * *",
        "0 9 * * *",
        "*/6 * * * *",
        "0 9 * * 1-5",
    ] {
        assert!(
            agent_job_too_frequent(&cron_agent_job(expr)).is_none(),
            "{expr} runs at most every 6 minutes and must not warn"
        );
    }
}

#[test]
fn a_cron_agent_job_under_five_minutes_is_still_caught() {
    for expr in [
        "*/3 * * * *",
        "*/1 * * * *",
        "*/4 * * * *",
        "*/30 * * * * *",
    ] {
        let hit = agent_job_too_frequent(&cron_agent_job(expr))
            .unwrap_or_else(|| panic!("{expr} runs oftener than every 5 minutes and must warn"));
        assert!(hit.gap() < MIN_AGENT_JOB_INTERVAL, "{expr}: {hit}");
    }
}

/// An irregular expression is judged by its tightest pair, whatever minute
/// the check happens to run at (CodeRabbit / Codex on #6159).
#[test]
fn an_irregular_cron_agent_job_is_judged_by_its_shortest_gap() {
    for expr in ["1,2,30 * * * *", "0,1 * * * *", "0,3 9 * * *"] {
        let hit = agent_job_too_frequent(&cron_agent_job(expr))
            .unwrap_or_else(|| panic!("{expr} has two runs inside five minutes and must warn"));
        assert!(hit.gap() <= ChronoDuration::minutes(3), "{expr}: {hit}");
    }
}

/// Exactly five minutes is the boundary the message names, and it is not
/// over it. `*/5` is also the expression the codebase's own tests reach for,
/// so warning on it would train an operator to ignore the warning.
#[test]
fn a_cron_agent_job_at_exactly_five_minutes_is_not_high_frequency() {
    assert!(agent_job_too_frequent(&cron_agent_job("*/5 * * * *")).is_none());
    let every_five = agent_job_on(Schedule::Every {
        every_ms: 5 * 60 * 1000,
    });
    assert!(agent_job_too_frequent(&every_five).is_none());
}

/// A schedule the arm cannot read is not evidence of anything, least of all
/// of running too often.
#[test]
fn an_unparseable_cron_expression_is_not_high_frequency() {
    assert!(agent_job_too_frequent(&cron_agent_job("not a cron expression")).is_none());
}
