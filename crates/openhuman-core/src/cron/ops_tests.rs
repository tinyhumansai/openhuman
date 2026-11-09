use super::*;
use tempfile::TempDir;
use tinyflows_bus::schedule::types::DeliveryConfig;

pub(super) fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn make_job(config: &Config, expr: &str, tz: Option<&str>, cmd: &str) -> CronJob {
    add_shell_job(
        config,
        None,
        Schedule::Cron {
            expr: expr.into(),
            tz: tz.map(Into::into),
            active_hours: None,
        },
        cmd,
    )
    .unwrap()
}

// ── add_once ────────────────────────────────────────────────────

#[test]
fn add_once_creates_future_at_schedule() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // The first call in a process starts the module runtime and attaches the
    // module (seconds); take the baseline after it so only the call is timed.
    crate::cron::list_jobs(&config).unwrap();
    let before = chrono::Utc::now();
    let job = add_once(&config, "5m", "echo hello").unwrap();
    match job.schedule {
        Schedule::At { at } => {
            let min = before + chrono::Duration::minutes(5) - chrono::Duration::seconds(2);
            let max = before + chrono::Duration::minutes(5) + chrono::Duration::seconds(5);
            assert!(at > min && at < max, "scheduled 'at' should land ~5m out");
        }
        other => panic!("expected At schedule, got {other:?}"),
    }
    assert_eq!(job.command, "echo hello");
}

#[test]
fn add_once_propagates_parse_delay_errors() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert!(add_once(&config, "", "cmd").is_err());
    assert!(add_once(&config, "5x", "cmd").is_err());
}

// ── add_once_at ─────────────────────────────────────────────────

#[test]
fn add_once_at_stores_exact_timestamp() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    // The store keeps instants as epoch milliseconds.
    let when = chrono::DateTime::from_timestamp_millis(
        (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp_millis(),
    )
    .unwrap();
    let job = add_once_at(&config, when, "echo hi").unwrap();
    match job.schedule {
        Schedule::At { at } => assert_eq!(at, when),
        other => panic!("expected At schedule, got {other:?}"),
    }
}

// ── pause_job / resume_job ──────────────────────────────────────

#[test]
fn pause_and_resume_toggle_enabled_flag() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = make_job(&config, "*/5 * * * *", None, "echo test");
    assert!(job.enabled);

    let paused = pause_job(&config, &job.id).unwrap();
    assert!(!paused.enabled);

    let resumed = resume_job(&config, &job.id).unwrap();
    assert!(resumed.enabled);
}

// ── cron_list / cron_update / cron_remove / cron_runs ───────────

fn disabled_cron_config(tmp: &TempDir) -> Config {
    let mut config = test_config(tmp);
    config.cron.enabled = false;
    config
}

#[tokio::test]
async fn cron_list_errors_when_cron_disabled() {
    let tmp = TempDir::new().unwrap();
    let config = disabled_cron_config(&tmp);
    let err = cron_list(&config).await.unwrap_err();
    assert!(err.contains("cron is disabled"));
}

#[tokio::test]
async fn cron_list_returns_jobs_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = make_job(&config, "*/5 * * * *", None, "echo test");
    let out = cron_list(&config).await.unwrap();
    assert!(out.value.iter().any(|j| j.id == job.id));
    assert!(out.logs.iter().any(|l| l.contains("cron jobs listed")));
}

#[tokio::test]
async fn cron_update_rejects_empty_job_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = cron_update(&config, "   ", CronJobPatch::default())
        .await
        .unwrap_err();
    assert!(err.contains("Missing 'job_id'"));
}

#[tokio::test]
async fn cron_update_errors_when_cron_disabled() {
    let tmp = TempDir::new().unwrap();
    let config = disabled_cron_config(&tmp);
    let err = cron_update(&config, "some-id", CronJobPatch::default())
        .await
        .unwrap_err();
    assert!(err.contains("cron is disabled"));
}

#[tokio::test]
async fn cron_update_mutates_existing_job() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = make_job(&config, "*/5 * * * *", None, "echo test");
    let patch = CronJobPatch {
        name: Some("renamed".to_string()),
        ..CronJobPatch::default()
    };
    let out = cron_update(&config, &job.id, patch).await.unwrap();
    assert_eq!(out.value.name.as_deref(), Some("renamed"));
    assert!(out.logs.iter().any(|l| l.contains("cron job updated")));
}

#[tokio::test]
async fn cron_remove_rejects_empty_job_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = cron_remove(&config, "").await.unwrap_err();
    assert!(err.contains("Missing 'job_id'"));
}

#[tokio::test]
async fn cron_remove_errors_when_cron_disabled() {
    let tmp = TempDir::new().unwrap();
    let config = disabled_cron_config(&tmp);
    let err = cron_remove(&config, "abc").await.unwrap_err();
    assert!(err.contains("cron is disabled"));
}

#[tokio::test]
async fn cron_remove_returns_removed_true_on_success() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = make_job(&config, "*/5 * * * *", None, "echo test");
    let out = cron_remove(&config, &job.id).await.unwrap();
    assert_eq!(out.value["job_id"], json!(job.id));
    assert_eq!(out.value["removed"], json!(true));
}

#[tokio::test]
async fn cron_runs_rejects_empty_job_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = cron_runs(&config, "", None).await.unwrap_err();
    assert!(err.contains("Missing 'job_id'"));
}

#[tokio::test]
async fn cron_runs_errors_when_cron_disabled() {
    let tmp = TempDir::new().unwrap();
    let config = disabled_cron_config(&tmp);
    let err = cron_runs(&config, "abc", Some(5)).await.unwrap_err();
    assert!(err.contains("cron is disabled"));
}

#[tokio::test]
async fn cron_runs_returns_empty_history_for_new_job() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = make_job(&config, "*/5 * * * *", None, "echo test");
    let out = cron_runs(&config, &job.id, Some(10)).await.unwrap();
    assert!(out.value.is_empty());
    assert!(out.logs.iter().any(|l| l.contains("cron run history")));
}

// ── cron_run ────────────────────────────────────────────────────

#[tokio::test]
async fn cron_run_rejects_empty_job_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = cron_run(&crate::core::runtime::DispatchCtx::ambient(), &config, "")
        .await
        .unwrap_err();
    assert!(err.contains("Missing 'job_id'"));
}

#[tokio::test]
async fn cron_run_rejects_whitespace_job_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let err = cron_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "   ",
    )
    .await
    .unwrap_err();
    assert!(err.contains("Missing 'job_id'"));
}

#[tokio::test]
async fn cron_run_errors_when_cron_disabled() {
    let tmp = TempDir::new().unwrap();
    let config = disabled_cron_config(&tmp);
    let err = cron_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "some-id",
    )
    .await
    .unwrap_err();
    assert!(err.contains("cron is disabled"));
}

#[tokio::test]
async fn cron_run_returns_queued_immediately_for_valid_job() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = make_job(&config, "*/5 * * * *", None, "echo hello");

    let out = cron_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &job.id,
    )
    .await
    .unwrap();
    assert_eq!(out.value["job_id"], json!(job.id));
    assert_eq!(out.value["status"], json!("queued"));
    assert!(out.logs.iter().any(|l| l.contains("enqueued")));
}

/// Guards the divergence reported in #6070: the controller schema declared
/// `status: ok|error` plus `duration_ms` and `output`, none of which the
/// handler can produce. Nothing validates a controller's *outputs* at runtime
/// (`validate_params` covers inputs only), so a mismatch is silent — it only
/// shows up in a generated client, the `/schema` catalog or the tool registry.
/// This asserts the two against each other so drift on either side fails here.
#[tokio::test]
async fn cron_run_response_conforms_to_its_declared_schema() {
    use crate::core::TypeSchema;
    use crate::cron::cron_schemas;

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = make_job(&config, "*/5 * * * *", None, "echo hello");

    let out = cron_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &job.id,
    )
    .await
    .unwrap();
    let result = out.value.as_object().expect("result payload is an object");

    let schema = cron_schemas("run");
    let TypeSchema::Object { fields } = &schema.outputs[0].ty else {
        panic!("cron_run must declare an object result");
    };

    for field in fields.iter().filter(|f| f.required) {
        assert!(
            result.contains_key(field.name),
            "schema declares required field '{}' that cron_run never returns; got {:?}",
            field.name,
            result.keys().collect::<Vec<_>>()
        );
    }

    for key in result.keys() {
        assert!(
            fields.iter().any(|f| f.name == key),
            "cron_run returns '{key}' but the schema does not declare it"
        );
    }

    // A present field must also match its declared *type*. Skipping a
    // non-string value here would let `{"job_id": 1, "status": 0}` pass the
    // two loops above, which is the same class of silent divergence this test
    // exists to catch.
    for field in fields {
        let Some(value) = result.get(field.name) else {
            continue; // absence is the required-field loop's business
        };
        match &field.ty {
            TypeSchema::String => assert!(
                value.is_string(),
                "cron_run returned {value} for '{}', which is declared String",
                field.name
            ),
            TypeSchema::Enum { variants } => {
                let text = value.as_str().unwrap_or_else(|| {
                    panic!(
                        "cron_run returned {value} for '{}', which is declared an enum of {variants:?}",
                        field.name
                    )
                });
                assert!(
                    variants.contains(&text),
                    "cron_run returned '{text}' for '{}', not among the declared variants {variants:?}",
                    field.name
                );
            }
            _ => {}
        }
    }
}

// ── run_job_now / single-flight ─────────────────────────────────

pub(super) fn single_flight_system_job(config: &Config, name: &str) -> CronJob {
    let job = crate::cron::system_jobs::ensure_system_job(
        config,
        name,
        Schedule::Every {
            every_ms: 3_600_000,
        },
    )
    .unwrap();
    crate::cron::update_job(
        config,
        &job.id,
        CronJobPatch {
            policy: Some(tinyflows_bus::schedule::JobPolicy {
                retries: Some(0),
                single_flight: true,
            }),
            ..CronJobPatch::default()
        },
    )
    .unwrap();
    crate::cron::get_job(config, &job.id).unwrap()
}

#[tokio::test]
async fn run_job_now_runs_and_records_the_handler_result() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let _handler = crate::cron::system_job_handlers::register(
        "ops-run-now",
        std::sync::Arc::new(|_ctx| Box::pin(async { Err("nope".to_string()) })),
    );
    let job = single_flight_system_job(&config, "ops-run-now");
    let (success, output) = run_job_now(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &job.id,
    )
    .await
    .unwrap();
    assert!(!success);
    assert!(output.contains("nope"), "{output}");
    let runs = cron::list_runs(&config, &job.id, 10).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "error");
    assert_eq!(
        cron::get_job(&config, &job.id)
            .unwrap()
            .last_status
            .as_deref(),
        Some("error")
    );
}

#[tokio::test]
async fn run_job_now_reports_an_unknown_job() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    assert!(run_job_now(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "no-such-job"
    )
    .await
    .is_err());
}

#[cfg(feature = "channels")]
#[tokio::test]
async fn run_job_now_records_required_delivery_failure_as_error() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.channels_config.telegram = Some(crate::config::TelegramConfig {
        bot_token: "test-token".into(),
        chat_id: None,
        allowed_users: vec!["target".into()],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });
    delivery_tests::install_failing_telegram_bridge();
    let _handler = crate::cron::system_job_handlers::register(
        "ops-run-now-delivery-failure",
        std::sync::Arc::new(|_ctx| Box::pin(async { Ok(()) })),
    );
    let job = single_flight_system_job(&config, "ops-run-now-delivery-failure");
    cron::update_job(
        &config,
        &job.id,
        CronJobPatch {
            delivery: Some(DeliveryConfig {
                mode: "announce".into(),
                channel: Some("telegram".into()),
                to: Some("target".into()),
                best_effort: false,
            }),
            ..CronJobPatch::default()
        },
    )
    .unwrap();

    let (success, output) = run_job_now(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &job.id,
    )
    .await
    .unwrap();

    assert!(!success, "required delivery failure must fail Run Now");
    assert!(
        output.contains("completed"),
        "execution itself succeeded: {output}"
    );
    let runs = cron::list_runs(&config, &job.id, 10).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "error");
    assert_eq!(
        runs[0].delivery_status,
        Some(tinyflows_bus::schedule::types::DeliveryStatus::Failed)
    );
    assert_eq!(
        cron::get_job(&config, &job.id)
            .unwrap()
            .last_status
            .as_deref(),
        Some("error")
    );
}
