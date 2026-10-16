use super::*;
use std::collections::BTreeMap;
use tinyagents_harness::observability::trace_export::{SpanKind, SpanStatus};

#[tokio::test]
async fn external_backend_is_skipped_before_session_lookup() {
    let mut config = Config::default();
    config.api_url = Some("https://external.example.invalid/openai/v1".into());
    let spans = [TraceSpan {
        trace_id: "thread-one:turn-one".into(),
        span_id: "root".into(),
        parent_span_id: None,
        name: "agent.turn".into(),
        kind: SpanKind::Turn,
        start_unix_ms: 1_000,
        end_unix_ms: Some(2_000),
        status: SpanStatus::Ok,
        attributes: BTreeMap::new(),
        input: None,
        output: None,
    }];
    assert!(push_spans(&config, &spans).await.is_ok());
}

/// The push must honour the host's runtime proxy (and so its CA/TLS setup)
/// rather than build a bare client.
#[tokio::test]
async fn the_push_client_goes_through_the_runtime_proxy() {
    use crate::config::schema::{ProxyConfig, ProxyScope};
    use crate::config::{runtime_proxy_config, set_runtime_proxy_config};
    use std::io::Read;

    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).unwrap_or(0);
            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
        }
    });
    let previous = runtime_proxy_config();
    set_runtime_proxy_config(ProxyConfig {
        enabled: true,
        http_proxy: Some(format!("http://127.0.0.1:{port}")),
        scope: ProxyScope::Services,
        services: vec![OTLP_SERVICE_KEY.into()],
        ..ProxyConfig::default()
    });
    let _ = otlp_client()
        .post("http://otlp.proxy-test.invalid/v1/traces")
        .body("{}")
        .send()
        .await;
    set_runtime_proxy_config(previous);
    let seen = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the proxy saw the request");
    assert!(
        seen.starts_with("POST http://otlp.proxy-test.invalid/v1/traces"),
        "{seen}"
    );
}
