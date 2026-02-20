pub mod engine;

#[cfg(test)]
mod tests {
    use crate::alphahuman::config::HeartbeatConfig;
    use crate::alphahuman::heartbeat::engine::HeartbeatEngine;
    use crate::alphahuman::observability::NoopObserver;
    use std::sync::Arc;

    #[test]
    fn heartbeat_engine_is_constructible_via_module_export() {
        let temp = tempfile::tempdir().unwrap();
        let engine = HeartbeatEngine::new(
            HeartbeatConfig::default(),
            temp.path().to_path_buf(),
            Arc::new(NoopObserver),
        );

        let _ = engine;
    }

    #[tokio::test]
    async fn ensure_heartbeat_file_creates_expected_file() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path();

        HeartbeatEngine::ensure_heartbeat_file(workspace)
            .await
            .unwrap();

        let heartbeat_path = workspace.join("HEARTBEAT.md");
        assert!(heartbeat_path.exists());
    }
}
