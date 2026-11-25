use super::*;

#[tokio::test]
async fn removes_retired_autonomy_cost_cap_from_persisted_config() {
    let tmp = TempDir::new().unwrap();
    let mut config = config_in(&tmp);
    config.schema_version = 14;
    config.save().await.unwrap();

    let persisted = fs::read_to_string(&config.config_path).unwrap();
    let persisted = persisted.replacen("[autonomy]", "[autonomy]\nmax_cost_per_day_cents = 321", 1);
    fs::write(&config.config_path, persisted).unwrap();

    run_pending(&mut config).await;

    assert_eq!(config.schema_version, 15);
    let persisted = fs::read_to_string(&config.config_path).unwrap();
    assert!(!persisted.contains("max_cost_per_day_cents"));
}
