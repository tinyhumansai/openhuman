use super::{model_matches, ConfiguredModelCheck};
use crate::config::Config;
use crate::doctor::runner;
use crate::doctor::{DoctorCheck, DoctorCtx, DoctorOptions, DoctorTier, Finding, FindingSeverity};

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

#[tokio::test]
async fn known_model_with_unsupported_dimensions_reports_the_dimension_path() {
    let mut config = Config::default();
    config.memory.embedding_provider = "openai".to_owned();
    config.memory.embedding_model = "text-embedding-3-small".to_owned();
    config.memory.embedding_dimensions = 1024;

    let findings = run_only(config).await;
    let result = finding(&findings);

    assert_eq!(result.severity, FindingSeverity::Error);
    assert_eq!(
        result.config_path.as_deref(),
        Some("memory.embedding_dimensions")
    );
}

#[tokio::test]
async fn active_embedding_route_dimensions_must_match_its_model() {
    use crate::config::EmbeddingRouteConfig;

    let mut config = Config::default();
    config.memory.embedding_model = "hint:recall".to_owned();
    config.memory.embedding_dimensions = 1024;
    config.embedding_routes = vec![EmbeddingRouteConfig {
        hint: "recall".to_owned(),
        provider: "openai".to_owned(),
        model: "text-embedding-3-small".to_owned(),
        dimensions: Some(1024),
    }];

    let findings = run_only(config).await;
    let paths = findings
        .iter()
        .filter(|result| result.check_id == EMBEDDINGS_CONFIG)
        .filter_map(|result| result.config_path.as_deref())
        .collect::<Vec<_>>();

    assert!(
        paths.contains(&"embedding_routes[0].dimensions"),
        "missing unsupported active route dimension finding: {findings:#?}"
    );
}

#[tokio::test]
async fn embedding_workload_model_must_match_memory_model() {
    let mut config = Config::default();
    config.memory.embedding_provider = "openai".to_owned();
    config.memory.embedding_model = "text-embedding-3-large".to_owned();
    config.memory.embedding_dimensions = 3072;
    config.embeddings_provider = Some("openai:text-embedding-3-small".to_owned());

    let findings = run_only(config).await;
    let result = finding(&findings);

    assert_eq!(result.severity, FindingSeverity::Error);
    assert_eq!(result.config_path.as_deref(), Some("embeddings_provider"));
}

#[tokio::test]
async fn configured_ollama_model_uses_each_selected_config_base_url() {
    let (url_a, request_a) = fake_ollama(200, r#"{"models":[{"name":"bge-m3:latest"}]}"#).await;
    let (url_b, request_b) = fake_ollama(200, r#"{"models":[{"name":"nomic-embed-text"}]}"#).await;

    let mut config_a = Config::default();
    config_a.memory.embedding_provider = "ollama".to_owned();
    config_a.memory.embedding_model = "bge-m3".to_owned();
    config_a.local_ai.base_url = Some(url_a);
    let findings_a = ConfiguredModelCheck
        .detect(&DoctorCtx::from_config(config_a))
        .await;

    let mut config_b = Config::default();
    config_b.memory.embedding_provider = "ollama".to_owned();
    config_b.memory.embedding_model = "bge-m3".to_owned();
    config_b.local_ai.base_url = Some(url_b);
    let findings_b = ConfiguredModelCheck
        .detect(&DoctorCtx::from_config(config_b))
        .await;

    assert!(findings_a.iter().any(|finding| {
        finding.check_id == "embeddings/configured-model"
            && finding.severity == FindingSeverity::Pass
    }));
    assert!(findings_b.iter().any(|finding| {
        finding.check_id == "embeddings/configured-model"
            && finding.severity == FindingSeverity::Error
    }));
    assert!(request_a
        .await
        .expect("fake Ollama A request")
        .starts_with("GET /api/tags "));
    assert!(request_b
        .await
        .expect("fake Ollama B request")
        .starts_with("GET /api/tags "));
}

#[tokio::test]
async fn non_ollama_embedding_provider_is_skipped_instead_of_reported_healthy() {
    let mut config = Config::default();
    config.memory.embedding_provider = "openai".to_owned();
    config.memory.embedding_model = "text-embedding-3-small".to_owned();

    let findings = ConfiguredModelCheck
        .detect(&DoctorCtx::from_config(config))
        .await;

    assert!(findings.iter().any(|finding| {
        finding.check_id == "embeddings/configured-model"
            && finding.severity == FindingSeverity::Skipped
    }));
    assert!(findings
        .iter()
        .all(|finding| finding.severity != FindingSeverity::Pass));
}

#[tokio::test]
async fn a_custom_embedding_route_rejects_non_http_urls() {
    use crate::config::EmbeddingRouteConfig;

    let mut config = Config::default();
    config.memory.embedding_model = "hint:custom".to_owned();
    config.embedding_routes = vec![EmbeddingRouteConfig {
        hint: "custom".to_owned(),
        provider: "custom:file:///private/model".to_owned(),
        model: "private-model-name".to_owned(),
        dimensions: Some(768),
    }];

    let findings = run_only(config).await;

    assert!(findings.iter().any(|finding| {
        finding.check_id == EMBEDDINGS_CONFIG
            && finding.severity == FindingSeverity::Error
            && finding.config_path.as_deref() == Some("embedding_routes[0].provider")
    }));
    assert!(findings.iter().all(|finding| {
        !finding.message.contains("private-model-name")
            && !finding.message.contains("/private/model")
    }));
}

#[tokio::test]
async fn custom_embedding_route_accepts_a_well_formed_http_url() {
    use crate::config::EmbeddingRouteConfig;

    let mut config = Config::default();
    config.memory.embedding_model = "hint:custom".to_owned();
    config.embedding_routes = vec![EmbeddingRouteConfig {
        hint: "custom".to_owned(),
        provider: "custom:https://embedding.example.test/v1".to_owned(),
        model: "custom-model".to_owned(),
        dimensions: Some(768),
    }];

    let findings = run_only(config).await;

    assert!(!findings.iter().any(|finding| {
        finding.check_id == EMBEDDINGS_CONFIG
            && finding.config_path.as_deref() == Some("embedding_routes[0].provider")
    }));
}

#[tokio::test]
async fn the_disabled_embedding_provider_is_a_valid_route_value() {
    use crate::config::EmbeddingRouteConfig;

    let mut config = Config::default();
    config.memory.embedding_model = "hint:disabled".to_owned();
    config.embedding_routes = vec![EmbeddingRouteConfig {
        hint: "disabled".to_owned(),
        provider: "none".to_owned(),
        model: "unused".to_owned(),
        dimensions: Some(768),
    }];

    let findings = run_only(config).await;

    assert!(!findings.iter().any(|finding| {
        finding.check_id == EMBEDDINGS_CONFIG
            && finding.config_path.as_deref() == Some("embedding_routes[0].provider")
    }));
}

#[tokio::test]
async fn incomplete_embedding_routes_keep_their_field_specific_findings() {
    use crate::config::EmbeddingRouteConfig;

    let mut config = Config::default();
    config.embedding_routes = vec![EmbeddingRouteConfig {
        hint: String::new(),
        provider: "openai".to_owned(),
        model: String::new(),
        dimensions: Some(0),
    }];

    let findings = run_only(config).await;
    let paths = findings
        .iter()
        .filter(|finding| finding.check_id == EMBEDDINGS_CONFIG)
        .filter_map(|finding| finding.config_path.as_deref())
        .collect::<Vec<_>>();

    for path in [
        "embedding_routes[0].hint",
        "embedding_routes[0].model",
        "embedding_routes[0].dimensions",
    ] {
        assert!(
            paths.contains(&path),
            "missing {path} finding: {findings:#?}"
        );
    }
    assert!(findings.iter().all(|finding| {
        finding.check_id != EMBEDDINGS_CONFIG
            || (!finding.message.contains("private") && !finding.fix_hint.trim().is_empty())
    }));
}

#[tokio::test]
async fn an_unresolved_embedding_hint_is_reported_without_echoing_the_hint() {
    let mut config = Config::default();
    config.memory.embedding_model = "hint:private-workload-hint".to_owned();
    config.embedding_routes.clear();

    let findings = run_only(config).await;

    assert!(findings.iter().any(|finding| {
        finding.check_id == EMBEDDINGS_CONFIG
            && finding.severity == FindingSeverity::Error
            && finding.config_path.as_deref() == Some("memory.embedding_model")
    }));
    assert!(findings.iter().all(|finding| {
        !finding.message.contains("private-workload-hint")
            && !finding.fix_hint.contains("private-workload-hint")
    }));
}

#[tokio::test]
async fn custom_embedding_route_rejects_empty_and_malformed_urls() {
    use crate::config::EmbeddingRouteConfig;

    for provider in ["custom:   ", "custom:not a url"] {
        let mut config = Config::default();
        config.memory.embedding_model = "hint:custom".to_owned();
        config.embedding_routes = vec![EmbeddingRouteConfig {
            hint: "custom".to_owned(),
            provider: provider.to_owned(),
            model: "custom-model".to_owned(),
            dimensions: Some(768),
        }];

        let findings = run_only(config).await;

        assert!(
            findings.iter().any(|finding| {
                finding.check_id == EMBEDDINGS_CONFIG
                    && finding.severity == FindingSeverity::Error
                    && finding.config_path.as_deref() == Some("embedding_routes[0].provider")
            }),
            "invalid endpoint was not reported: {findings:#?}"
        );
    }
}

#[test]
fn configured_ollama_model_matches_exact_and_tagged_names() {
    assert!(model_matches("bge-m3", "bge-m3"));
    assert!(model_matches("bge-m3:latest", "bge-m3"));
    assert!(model_matches("bge-m3", "bge-m3:latest"));
    assert!(model_matches("bge-m3:v1.0", "bge-m3"));
    assert!(!model_matches("nomic-embed-text:latest", "bge-m3"));
    assert!(!model_matches("bge-m3:latest", "nomic-embed-text"));
    assert!(!model_matches("bge-m3:latest", "bge-m3:v1.0"));
}

async fn fake_ollama(status: u16, body: &'static str) -> (String, tokio::task::JoinHandle<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local fake Ollama");
    let address = listener.local_addr().expect("fake Ollama address");
    let request = tokio::spawn(async move {
        let (mut stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                .await
                .expect("configured Ollama probe should reach the local fixture")
                .expect("accept local probe");
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 512];
        loop {
            let read = stream.read(&mut buffer).await.expect("read HTTP request");
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let reason = if status == 200 {
            "OK"
        } else {
            "Service Unavailable"
        };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write fake Ollama response");
        String::from_utf8_lossy(&bytes).into_owned()
    });

    (format!("http://{address}"), request)
}
