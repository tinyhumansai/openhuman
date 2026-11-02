//! A corrupt config is logged by its safe summary, never by the parser's text,
//! which quotes the offending source line (and so any secret on it).

use super::*;

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

const LEAKY: &str = "api_key = \"sk-live-SECRETVALUE\n";

async fn logs_of_recovery(backup: Option<&str>) -> String {
    let logs = Logs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();

    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");
    std::fs::write(&config_path, LEAKY).unwrap();
    if let Some(backup) = backup {
        std::fs::write(config_path.with_extension("toml.bak"), backup).unwrap();
    }
    let (_, corrupted) = parse_config_with_recovery(&config_path, LEAKY).await;
    assert!(corrupted);
    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    text
}

#[tokio::test]
async fn corrupt_config_warn_log_omits_values() {
    for backup in [None, Some(LEAKY)] {
        let text = logs_of_recovery(backup).await;
        assert!(
            text.contains("corrupted"),
            "{backup:?}: no corruption warning in {text:?}"
        );
        assert!(!text.contains("SECRETVALUE"), "{backup:?}: {text}");
    }
}
