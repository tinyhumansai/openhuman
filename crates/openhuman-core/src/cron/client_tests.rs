use super::*;
use chrono::TimeZone;

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2030, 1, 1, hour, 0, 0).unwrap()
}

fn cron(expr: &str) -> Schedule {
    Schedule::Cron {
        expr: expr.into(),
        tz: Some("UTC".into()),
        active_hours: None,
    }
}

#[tokio::test]
async fn next_run_comes_from_the_module() {
    let config = Config::default();
    let next = next_run(&config, &cron("0 9 * * *"), at(8)).await.unwrap();
    assert_eq!(next, at(9));
    assert!(next_run(&config, &cron("not a cron"), at(8)).await.is_err());
}

#[tokio::test]
async fn schedules_are_validated_and_the_agent_floor_applies() {
    let config = Config::default();
    validate_schedule(&config, &cron("0 9 * * *"), at(8), true)
        .await
        .unwrap();
    let every_minute = Schedule::Every { every_ms: 60_000 };
    validate_schedule(&config, &every_minute, at(8), false)
        .await
        .unwrap();
    assert!(validate_schedule(&config, &every_minute, at(8), true)
        .await
        .is_err());
    let past = Schedule::At { at: at(1) };
    assert!(validate_schedule(&config, &past, at(8), false)
        .await
        .is_err());
}

#[tokio::test]
async fn expressions_normalise_and_report_their_cron_form() {
    let config = Config::default();
    assert_eq!(
        normalize_expression(&config, "0 9 * * 1-5").await.unwrap(),
        "0 0 9 * * 2-6"
    );
    assert!(schedule_cron_expression(&config, &cron("0 9 * * *"))
        .await
        .unwrap()
        .is_some());
    assert!(
        schedule_cron_expression(&config, &Schedule::Every { every_ms: 1000 })
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn delays_parse_and_delivery_defaults_follow_the_origin() {
    let config = Config::default();
    assert_eq!(
        parse_human_delay(&config, "2h").await.unwrap(),
        Duration::hours(2)
    );
    assert!(parse_human_delay(&config, "soon").await.is_err());
    assert_eq!(
        default_delivery(&config, None).await.unwrap().mode,
        "proactive"
    );
    let origin = JobOrigin::Web {
        thread_id: "t1".into(),
        agent_id: None,
    };
    assert_eq!(
        default_delivery(&config, Some(&origin)).await.unwrap().mode,
        "origin"
    );
}

#[tokio::test]
async fn origin_delivery_without_an_origin_is_refused() {
    let config = Config::default();
    let origin_delivery = default_delivery(
        &config,
        Some(&JobOrigin::Web {
            thread_id: "t1".into(),
            agent_id: None,
        }),
    )
    .await
    .unwrap();
    assert!(
        check_origin_requirements(&config, &SessionTarget::Isolated, &origin_delivery, None)
            .await
            .is_err()
    );
    check_origin_requirements(
        &config,
        &SessionTarget::Isolated,
        &DeliveryConfig::default(),
        None,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn long_output_is_cut_at_the_contract_bound() {
    let config = Config::default();
    let long = "x".repeat(tinyflows_bus::schedule::MAX_CRON_OUTPUT_BYTES * 2);
    let cut = truncate_cron_output(&config, &long).await.unwrap();
    assert!(cut.len() <= tinyflows_bus::schedule::MAX_CRON_OUTPUT_BYTES + 32);
    assert!(cut.ends_with(tinyflows_bus::schedule::TRUNCATED_OUTPUT_MARKER));
}

#[tokio::test]
async fn runs_closer_than_reports_the_tightest_pair() {
    let config = Config::default();
    let hit = runs_closer_than(&config, &cron("*/3 * * * *"), at(8), Duration::minutes(5))
        .await
        .unwrap();
    assert!(hit.is_some());
    let none = runs_closer_than(&config, &cron("0 9 * * *"), at(8), Duration::minutes(5))
        .await
        .unwrap();
    assert!(none.is_none());
}

#[tokio::test]
async fn a_module_error_keeps_its_own_message() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let error = store::<CronJob>(
        &config,
        StoreOperation::Get {
            job_id: "missing".into(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "Cron job 'missing' not found");
}

#[tokio::test]
async fn a_switched_off_component_answers_component_disabled() {
    let mut config = Config::default();
    config.components.tinyflows = false;
    let error = parse_human_delay(&config, "1h").await.unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("COMPONENT_DISABLED:tinyflows"),
        "{error}"
    );
}
