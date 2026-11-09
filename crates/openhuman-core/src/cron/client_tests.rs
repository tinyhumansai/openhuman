use super::*;
use tinyflows_bus::schedule::CronJob;

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
