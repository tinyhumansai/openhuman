use crate::config::Config;
use crate::doctor::runner;
use crate::doctor::{DoctorCtx, DoctorOptions, DoctorTier, Finding, FindingSeverity};

const EMBEDDINGS_CONFIG: &str = "embeddings/config";

async fn run_only(config: Config) -> Vec<Finding> {
    let context = DoctorCtx::from_config(config);
    runner::run(
        &context,
        DoctorOptions {
            tier: DoctorTier::Static,
            only: vec![EMBEDDINGS_CONFIG.to_owned()],
            ..DoctorOptions::default()
        },
    )
    .await
    .expect("the selected doctor check should run")
    .findings
}

fn finding(findings: &[Finding]) -> &Finding {
    findings
        .iter()
        .find(|finding| finding.check_id == EMBEDDINGS_CONFIG)
        .unwrap_or_else(|| panic!("expected finding from {EMBEDDINGS_CONFIG}: {findings:#?}"))
}

#[tokio::test]
async fn an_unknown_embedding_provider_reports_its_config_path() {
    let mut config = Config::default();
    config.memory.embedding_provider = "nope".to_owned();

    let findings = run_only(config).await;
    let result = finding(&findings);

    assert_eq!(result.severity, FindingSeverity::Error);
    assert_eq!(
        result.config_path.as_deref(),
        Some("memory.embedding_provider")
    );
}

#[tokio::test]
async fn missing_embedding_model_and_dimensions_are_reported() {
    let mut config = Config::default();
    config.memory.embedding_model.clear();
    config.memory.embedding_dimensions = 0;

    let findings = run_only(config).await;
    let paths = findings
        .iter()
        .filter(|result| result.check_id == EMBEDDINGS_CONFIG)
        .filter_map(|result| result.config_path.as_deref())
        .collect::<Vec<_>>();

    assert!(
        paths.contains(&"memory.embedding_model"),
        "missing model path in findings: {findings:#?}"
    );
    assert!(
        paths.contains(&"memory.embedding_dimensions"),
        "missing dimensions path in findings: {findings:#?}"
    );
    assert!(findings
        .iter()
        .filter(|result| result.check_id == EMBEDDINGS_CONFIG)
        .all(|result| result.severity == FindingSeverity::Error));
}
