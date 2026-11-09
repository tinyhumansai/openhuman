use super::*;

#[tokio::test]
async fn private_agent_progress_advances_cursor_without_leaking_text_or_replaying_terminal() {
    use crate::core::events::DomainEvent;
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;
    use tinybus::EventHandler;

    #[derive(Default)]
    struct Collector {
        finished: Arc<StdMutex<Vec<(String, String)>>>,
        progress: Arc<StdMutex<Vec<String>>>,
    }

    #[async_trait]
    impl EventHandler<DomainEvent> for Collector {
        fn name(&self) -> &str {
            "test::flows::poll_run_projection_collector"
        }

        fn domains(&self) -> Option<&[&str]> {
            Some(&["cron"])
        }

        async fn handle(&self, event: &DomainEvent) {
            match event {
                DomainEvent::FlowRunFinished { run_id, status, .. } => self
                    .finished
                    .lock()
                    .unwrap()
                    .push((run_id.clone(), status.clone())),
                DomainEvent::FlowRunProgress { run_id, status, .. } => self
                    .progress
                    .lock()
                    .unwrap()
                    .push(format!("{run_id}:{status}")),
                _ => {}
            }
        }
    }

    crate::core::bus::init().await.expect("bus init");
    let collector = Arc::new(Collector::default());
    let handler: Arc<dyn EventHandler<DomainEvent>> = collector.clone();
    let _subscription = crate::core::bus::BUS
        .subscribe(handler)
        .expect("subscriber installed");

    let tmp = tempfile::TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let run_id = format!("flow:projection:{}", uuid::Uuid::new_v4());
    let flow = Flow {
        id: format!("projection:{}", uuid::Uuid::new_v4()),
        name: "Projection test".into(),
        enabled: true,
        graph: serde_json::json!({"nodes": [], "edges": []}),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        last_run_at: None,
        last_status: None,
        require_approval: false,
    };
    let ctx = crate::core::runtime::DispatchCtx::host_default();
    let bindings = super::super::register_bindings(&ctx, &config, &flow, &run_id)
        .await
        .expect("scoped run callbacks registered");
    let session = RunSession::new(bindings, Arc::new(config), flow, FlowRunTrigger::Rpc, 0);

    let agent_progress = RunEvent {
        seq: 1,
        kind: "agent_progress".into(),
        data: serde_json::json!({
            "node_id": "agent-node",
            "event_kind": "text_delta",
            "text": "private agent output must stay off DomainEvent",
            "agent_seq": 1
        }),
        graph_run_id: None,
    };
    assert!(session.consume(&agent_progress, true).await);
    assert_eq!(session.cursor(), 1);
    assert!(collector.progress.lock().unwrap().is_empty());

    let finished = RunEvent {
        seq: 2,
        kind: "finished".into(),
        data: serde_json::json!({
            "status": "completed",
            "successful": true,
            "error": null,
            "graph_run_id": null,
            "output": {"ok": true},
            "timed_out": false
        }),
        graph_run_id: None,
    };
    assert!(session.consume(&finished, true).await);
    assert_eq!(session.cursor(), 2);
    assert!(!session.consume(&finished, true).await);

    for _ in 0..20 {
        if collector
            .finished
            .lock()
            .unwrap()
            .iter()
            .any(|(id, _)| id == &run_id)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let mut terminal_events = collector.finished.lock().unwrap().clone();
    terminal_events.retain(|(id, _)| id == &run_id);
    assert_eq!(terminal_events, [(run_id, "completed".into())]);
    assert!(collector.progress.lock().unwrap().is_empty());
}

#[test]
fn run_event_poller_survives_the_invocations_runtime() {
    let caller = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (release, wait) = tokio::sync::oneshot::channel();
    let (finished, result) = std::sync::mpsc::channel();
    caller.block_on(async {
        spawn_poller(async move {
            let _ = wait.await;
            let _ = finished.send(());
        })
        .unwrap();
    });
    drop(caller);
    let _ = release.send(());
    result
        .recv_timeout(Duration::from_secs(5))
        .expect("run event polling outlives the invocation runtime");
}
