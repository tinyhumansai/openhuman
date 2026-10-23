use super::*;

#[test]
fn migrate_legacy_inference_url_log_omits_credentials_and_path() {
    #[derive(Clone, Default)]
    struct Logs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Logs {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();

    let mut cfg = Config::default();
    cfg.api_url = Some(
        "https://alice:hunter2@api.openai.com/v1/private/sk-secret/chat/completions".to_owned(),
    );
    cfg.inference_url = None;
    migrate_legacy_inference_url(&mut cfg);

    let output = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(
        output.contains("[config][migrate] splitting legacy api_url"),
        "{output}"
    );
    for secret in ["alice", "hunter2", "/v1/private", "sk-secret"] {
        assert!(
            !output.contains(secret),
            "migration log leaked {secret}: {output}"
        );
    }
}
