use super::runner::{output_path, run_monitor, RunnerContext};
use super::store::global_store;
use super::types::*;
use crate::openhuman::agent::host_runtime::{NativeRuntime, RuntimeAdapter};
use crate::openhuman::inference::provider::thread_context::current_thread_id;
use crate::openhuman::security::{AuditLogger, CommandClass, GateDecision, SecurityPolicy};
use crate::rpc::RpcOutcome;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

pub(crate) fn classify_monitor_command(
    security: &SecurityPolicy,
    command: &str,
    declared_category: Option<&str>,
) -> (CommandClass, GateDecision) {
    let mut class = security.classify_command(command);
    if let Some(declared) = declared_category.and_then(SecurityPolicy::parse_declared_class) {
        class = class.max(declared);
    }
    let gate_decision = security.gate_decision(class);
    (class, gate_decision)
}

pub async fn start(
    request: MonitorStartRequest,
    security: Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    audit: Arc<AuditLogger>,
) -> Result<RpcOutcome<MonitorStartResponse>, String> {
    tracing::debug!(
        persistent = request.persistent,
        timeout_ms = request.timeout_ms,
        category = request.category.as_deref().unwrap_or(""),
        "[monitor] ops:start entry"
    );
    let command = request.command.trim();
    if command.is_empty() {
        tracing::debug!("[monitor] ops:start rejected empty command");
        return Err("command is required".to_string());
    }
    let (class, gate_decision) =
        classify_monitor_command(&security, command, request.category.as_deref());
    tracing::debug!(
        class = ?class,
        gate_decision = ?gate_decision,
        "[monitor] ops:start classified command"
    );
    if gate_decision == GateDecision::Block {
        security.check_gated_command(command).map_err(|reason| {
            tracing::debug!(class = ?class, reason = %reason, "[monitor] ops:start blocked");
            reason.to_string()
        })?;
    }
    security.check_gated_command(command).map_err(|reason| {
        tracing::debug!(class = ?class, reason = %reason, "[monitor] ops:start denied");
        reason.to_string()
    })?;
    if security.is_rate_limited() || !security.record_action() {
        tracing::debug!("[monitor] ops:start rate limited");
        return Err("Rate limit exceeded: action budget exhausted".to_string());
    }

    let monitor_id = format!("mon_{}", uuid::Uuid::new_v4().simple());
    let output_file = output_path(&security.workspace_dir, &monitor_id);
    if let Some(parent) = output_file.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create monitor output directory: {e}"))?;
    }
    let parent = crate::openhuman::agent::harness::current_parent();
    let thread_id = current_thread_id();
    let session_id = parent.as_ref().map(|p| p.session_id.clone());
    tracing::debug!(
        monitor_id = %monitor_id,
        thread_id = thread_id.as_deref().unwrap_or(""),
        session_id = session_id.as_deref().unwrap_or(""),
        output_file = %output_file.display(),
        "[monitor] ops:start creating snapshot"
    );
    let now = now_ms();
    let description = request
        .description
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| command.chars().take(80).collect());
    let snapshot = MonitorSnapshot {
        monitor_id: monitor_id.clone(),
        status: MonitorStatus::Starting,
        description: description.clone(),
        command: command.to_string(),
        output_file: output_file.clone(),
        persistent: request.persistent,
        thread_id: thread_id.clone(),
        session_id: session_id.clone(),
        started_at_ms: now,
        updated_at_ms: now,
        exit_code: None,
        error: None,
        output_bytes: 0,
        dropped_bytes: 0,
        recent_events: Vec::new(),
    };
    let (stop_tx, stop_rx) = oneshot::channel();
    let store = global_store();
    store.insert(snapshot.clone(), stop_tx).await;
    let timeout_ms = request
        .timeout_ms
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(1, MAX_TIMEOUT_MS);
    let ctx = RunnerContext {
        security,
        runtime,
        audit,
        store,
        run_queue: parent.and_then(|p| p.run_queue),
    };
    tokio::spawn(run_monitor(
        ctx,
        snapshot,
        Duration::from_millis(timeout_ms),
        stop_rx,
    ));
    tracing::debug!(
        monitor_id = %monitor_id,
        thread_id = thread_id.as_deref().unwrap_or(""),
        session_id = session_id.as_deref().unwrap_or(""),
        timeout_ms,
        "[monitor] ops:start exit running"
    );
    Ok(RpcOutcome::new(
        MonitorStartResponse {
            monitor_id,
            status: MonitorStatus::Running,
            description,
            output_file,
            persistent: request.persistent,
        },
        vec![],
    ))
}

pub async fn start_default(
    request: MonitorStartRequest,
) -> Result<RpcOutcome<MonitorStartResponse>, String> {
    tracing::debug!("[monitor] ops:start_default loading config");
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| {
            tracing::debug!(error = %e, "[monitor] ops:start_default config load failed");
            format!("failed to load config: {e}")
        })?;
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    tracing::debug!(
        workspace_dir = %config.workspace_dir.display(),
        action_dir = %config.action_dir.display(),
        "[monitor] ops:start_default created security policy"
    );
    start(
        request,
        security,
        Arc::new(NativeRuntime::new()),
        AuditLogger::disabled(),
    )
    .await
}

pub async fn list() -> Result<RpcOutcome<MonitorListResponse>, String> {
    tracing::debug!("[monitor] ops:list entry");
    let monitors = global_store().list().await;
    tracing::debug!(count = monitors.len(), "[monitor] ops:list exit");
    Ok(RpcOutcome::new(MonitorListResponse { monitors }, vec![]))
}

pub async fn stop(request: MonitorStopRequest) -> Result<RpcOutcome<MonitorStopResponse>, String> {
    tracing::debug!(monitor_id = %request.monitor_id, "[monitor] ops:stop entry");
    let snapshot = global_store()
        .stop(&request.monitor_id)
        .await
        .map_err(|error| {
            tracing::debug!(monitor_id = %request.monitor_id, %error, "[monitor] ops:stop failed");
            error
        })?;
    tracing::debug!(
        monitor_id = %snapshot.monitor_id,
        status = ?snapshot.status,
        "[monitor] ops:stop exit"
    );
    Ok(RpcOutcome::new(
        MonitorStopResponse {
            monitor_id: snapshot.monitor_id,
            status: snapshot.status,
        },
        vec![],
    ))
}

pub async fn read(request: MonitorReadRequest) -> Result<RpcOutcome<MonitorReadResponse>, String> {
    tracing::debug!(
        monitor_id = %request.monitor_id,
        max_bytes = request.max_bytes,
        "[monitor] ops:read entry"
    );
    let path = global_store()
        .output_file(&request.monitor_id)
        .await
        .ok_or_else(|| {
            tracing::debug!(monitor_id = %request.monitor_id, "[monitor] ops:read missing monitor");
            format!("monitor `{}` not found", request.monitor_id)
        })?;
    let bytes = tokio::fs::read(&path).await.map_err(|e| {
        tracing::debug!(
            monitor_id = %request.monitor_id,
            output_file = %path.display(),
            error = %e,
            "[monitor] ops:read file read failed"
        );
        format!("failed to read monitor output: {e}")
    })?;
    let max = request.max_bytes.unwrap_or(64 * 1024).max(1);
    let truncated = bytes.len() > max;
    let slice = if truncated {
        &bytes[bytes.len() - max..]
    } else {
        bytes.as_slice()
    };
    tracing::debug!(
        monitor_id = %request.monitor_id,
        output_file = %path.display(),
        bytes = slice.len(),
        truncated,
        "[monitor] ops:read exit"
    );
    Ok(RpcOutcome::new(
        MonitorReadResponse {
            monitor_id: request.monitor_id,
            output: String::from_utf8_lossy(slice).to_string(),
            truncated,
            bytes: slice.len(),
        },
        vec![],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::security::AutonomyLevel;

    static MONITOR_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn test_security(tmp: &tempfile::TempDir, autonomy: AutonomyLevel) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy,
            workspace_dir: tmp.path().to_path_buf(),
            action_dir: tmp.path().to_path_buf(),
            ..SecurityPolicy::default()
        })
    }

    #[tokio::test]
    async fn start_denies_write_in_read_only() {
        let _guard = MONITOR_TEST_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let result = start(
            MonitorStartRequest {
                command: "touch nope".into(),
                description: None,
                timeout_ms: Some(100),
                persistent: false,
                category: None,
            },
            test_security(&tmp, AutonomyLevel::ReadOnly),
            Arc::new(NativeRuntime::new()),
            AuditLogger::disabled(),
        )
        .await;
        assert!(result.is_err());
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn monitor_streams_and_reads_output() {
        let _guard = MONITOR_TEST_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let store = global_store();
        store.clear().await;
        let response = start(
            MonitorStartRequest {
                command: "printf 'one\\ntwo\\n'".into(),
                description: Some("test".into()),
                timeout_ms: Some(2_000),
                persistent: false,
                category: None,
            },
            test_security(&tmp, AutonomyLevel::Supervised),
            Arc::new(NativeRuntime::new()),
            AuditLogger::disabled(),
        )
        .await
        .unwrap()
        .value;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let read = read(MonitorReadRequest {
            monitor_id: response.monitor_id.clone(),
            max_bytes: Some(4096),
        })
        .await
        .unwrap()
        .value;
        assert!(read.output.contains("one"));
        assert!(read.output.contains("two"));
        let snapshot = store.get(&response.monitor_id).await.unwrap();
        assert!(!snapshot.recent_events.is_empty());
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn monitor_stop_marks_running_monitor_stopped() {
        let _guard = MONITOR_TEST_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let store = global_store();
        store.clear().await;
        let response = start(
            MonitorStartRequest {
                command: "sleep 5".into(),
                description: None,
                timeout_ms: Some(5_000),
                persistent: false,
                category: None,
            },
            test_security(&tmp, AutonomyLevel::Supervised),
            Arc::new(NativeRuntime::new()),
            AuditLogger::disabled(),
        )
        .await
        .unwrap()
        .value;
        let stopped = stop(MonitorStopRequest {
            monitor_id: response.monitor_id,
        })
        .await
        .unwrap()
        .value;
        assert_eq!(stopped.status, MonitorStatus::Stopped);
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn monitor_timeout_marks_status() {
        let _guard = MONITOR_TEST_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let store = global_store();
        store.clear().await;
        let response = start(
            MonitorStartRequest {
                command: "sleep 1".into(),
                description: None,
                timeout_ms: Some(50),
                persistent: false,
                category: None,
            },
            test_security(&tmp, AutonomyLevel::Supervised),
            Arc::new(NativeRuntime::new()),
            AuditLogger::disabled(),
        )
        .await
        .unwrap()
        .value;
        tokio::time::sleep(Duration::from_millis(150)).await;
        let snapshot = store.get(&response.monitor_id).await.unwrap();
        assert_eq!(snapshot.status, MonitorStatus::TimedOut);
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn monitor_output_is_bounded() {
        let _guard = MONITOR_TEST_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let store = global_store();
        store.clear().await;
        let response = start(
            MonitorStartRequest {
                command: "head -c 1200000 /dev/zero | tr '\\0' x".into(),
                description: None,
                timeout_ms: Some(2_000),
                persistent: false,
                category: None,
            },
            test_security(&tmp, AutonomyLevel::Supervised),
            Arc::new(NativeRuntime::new()),
            AuditLogger::disabled(),
        )
        .await
        .unwrap()
        .value;
        let mut snapshot = store.get(&response.monitor_id).await.unwrap();
        for _ in 0..50 {
            if snapshot.dropped_bytes > 0
                || matches!(
                    snapshot.status,
                    MonitorStatus::Completed | MonitorStatus::Failed | MonitorStatus::TimedOut
                )
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            snapshot = store.get(&response.monitor_id).await.unwrap();
        }
        assert!(snapshot.output_bytes <= MAX_OUTPUT_BYTES);
        assert!(
            snapshot.dropped_bytes > 0,
            "large monitor output should be dropped after the bound (output_bytes={}, dropped_bytes={}, status={:?})",
            snapshot.output_bytes,
            snapshot.dropped_bytes,
            snapshot.status
        );
    }
}
