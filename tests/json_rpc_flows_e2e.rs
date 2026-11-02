//! JSON-RPC E2E cases for the flows namespace and its published cron module worker.

#[path = "support/rpc_e2e_harness.rs"]
mod harness;
#[path = "support/published_tinyflows.rs"]
mod published_tinyflows;
use harness::*;

#[test]
fn json_rpc_cron_manual_worker_survives_caller_cancel_on_published_tinyflows() {
    const CHILD_ENV: &str = "OPENHUMAN_CRON_PUBLISHED_MODULE_E2E_CHILD";
    if std::env::var_os(CHILD_ENV).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "json_rpc_cron_manual_worker_survives_caller_cancel_on_published_tinyflows",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, "1")
            .output()
            .expect("run isolated published TinyFlows host E2E child");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        print!("{stdout}");
        eprint!("{stderr}");
        let child_output = format!("{stdout}{stderr}");
        assert!(
            output.status.success(),
            "published TinyFlows host E2E child failed:\n{child_output}"
        );
        assert!(
            child_output.contains("running 1 test")
                && child_output.contains(
                    "test json_rpc_cron_manual_worker_survives_caller_cancel_on_published_tinyflows ... ok"
                )
                && child_output.contains("1 passed; 0 failed"),
            "the child must prove exactly one test ran and passed:\n{child_output}"
        );
        return;
    }

    run_json_rpc_e2e_on_agent_stack(
        "json_rpc_cron_manual_worker_survives_caller_cancel_on_published_tinyflows",
        json_rpc_cron_manual_worker_survives_caller_cancel_on_published_tinyflows_inner,
    );
}

async fn json_rpc_cron_manual_worker_survives_caller_cancel_on_published_tinyflows_inner() {
    use openhuman_core::storage::{CollectionSpec, Precondition, Scope, StorageUrl};
    use openhuman_core::{
        config::Config,
        core::{
            events::DomainEvent,
            runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet, TokenSource},
            types::HostKind,
        },
        cron::system_job_handlers,
    };
    use tinybus::EventHandler;
    use tokio::sync::{mpsc, oneshot, Semaphore};

    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let workspace = home.join("cron-workspace");
    std::fs::create_dir_all(&workspace).expect("create E2E workspace");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _storage_url_guard = EnvVarGuard::unset("OPENHUMAN_STORAGE_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _source_guard = EnvVarGuard::unset("OPENHUMAN_MODULE_SOURCE");
    let _source_dir_guard = EnvVarGuard::unset("OPENHUMAN_MODULE_SOURCE_DIR");
    clear_forced_chat_completions();

    let (artifact, _) = published_tinyflows::staged_v0_9_4_module();
    let module_path = artifact.parent().expect("native artifact directory");
    let _module_path_guard = EnvVarGuard::set_to_path("OPENHUMAN_MODULE_PATH", module_path);

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{mock_addr}");
    write_min_config(&openhuman_home, &mock_origin);
    write_min_config(&openhuman_home.join("users").join("e2e-user"), &mock_origin);

    let storage_url = openhuman_core::storage::config::default_url(&workspace);
    let config_path = openhuman_home.join("config.toml");
    let config_text = std::fs::read_to_string(&config_path).expect("read E2E config");
    std::fs::write(
        &config_path,
        format!("{config_text}\n[storage]\nurl = {storage_url:?}\n"),
    )
    .expect("bind E2E config to its explicit storage URL");
    let mut config = Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace.clone(),
        config_path,
        ..Config::default()
    };
    config.storage.url = Some(storage_url.clone());

    // Open the exact configured backend without the host boot wrapper, which
    // runs composed schema migrations immediately. Seed the legacy v1 rows
    // first, then let normal CoreContext startup perform the real migration.
    let parsed_url = StorageUrl::parse(&storage_url).expect("parse the configured storage URL");
    let backend = tinystoragedrivers::open(&parsed_url)
        .await
        .expect("open the host's workspace storage backend");
    openhuman_core::storage::install_for_url(&storage_url, Arc::clone(&backend));
    assert_eq!(
        openhuman_core::storage::installed_url().as_deref(),
        Some(storage_url.as_str())
    );
    let scoped = backend
        .for_scope(&Scope::local())
        .expect("open the operator storage scope");
    let docs = Arc::clone(scoped.documents());
    for collection in ["cron_jobs", "cron_runs", "cron_counters", "tsd.migrations"] {
        docs.ensure_collection(&CollectionSpec::new(collection))
            .await
            .unwrap();
    }
    assert!(
        docs.get("tsd.migrations", "cron/cron.jobs/jobs_v2")
            .await
            .unwrap()
            .is_none(),
        "the fresh backend has not run Cron migration before the v1 fixture is seeded"
    );
    docs.put(
        "cron_jobs",
        "legacy-migration-proof",
        json!({
            "claim_generation": 1,
            "expression": "0 9 * * *",
            "command": "echo migrated",
            "schedule": "{\"kind\":\"every\",\"every_ms\":3600000}",
            "job_type": "shell",
            "session_target": "isolated",
            "enabled": false,
            "delivery": "{\"mode\":\"none\",\"channel\":null,\"to\":null,\"best_effort\":true}",
            "delete_after_run": false,
            "created_at": "2026-03-04T05:06:07.123Z",
            "created_ms": 1772600767123_i64,
            "next_run": "2026-03-05T09:00:00Z",
            "next_run_ms": 1772701200000_i64,
            "next_run_ns": 1772701200000000000_i64,
            "name": "legacy-migration-proof"
        }),
        Precondition::None,
    )
    .await
    .unwrap();
    docs.put(
        "cron_runs",
        "00000000000000000007",
        json!({
            "seq": 7,
            "job_id": "legacy-migration-proof",
            "started_at": "2026-03-04T09:00:00Z",
            "started_ns": 1772614800000000000_i64,
            "finished_at": "2026-03-04T09:00:02Z",
            "status": "success",
            "duration_ms": 2000,
            "output": "legacy run preserved"
        }),
        Precondition::None,
    )
    .await
    .unwrap();
    docs.put(
        "cron_counters",
        "runs",
        json!({ "next": 8 }),
        Precondition::None,
    )
    .await
    .unwrap();

    openhuman_core::core::bus::init().await.unwrap();
    let (default_context, _, _) = CoreContext::init_with_config(
        HostKind::Library,
        &TokenSource::Fixed(Arc::new(TEST_RPC_TOKEN.to_string())),
        DomainSet::full(),
        openhuman_core::tools::toolpacks::ToolGroups::none(),
        Some(config.clone()),
    )
    .await
    .expect("initialize the isolated core context for the native host scenario");
    assert_eq!(
        default_context.workspace_dir().unwrap(),
        workspace,
        "the RPC host and pre-seeded migration rows share the same workspace"
    );
    assert_eq!(
        openhuman_core::storage::installed_url().as_deref(),
        Some(storage_url.as_str()),
        "CoreContext initialization retains the seeded backend URL"
    );
    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");
    let jobs = post_json_rpc(&rpc_base, 80_001, "openhuman.cron_list", json!({})).await;
    let jobs = assert_no_jsonrpc_error(&jobs, "cron_list");
    assert!(
        jobs.to_string().contains("legacy-migration-proof"),
        "the first public Cron read runs the module migration: {jobs}"
    );
    let migrated = docs
        .get("cron.jobs", "legacy-migration-proof")
        .await
        .unwrap()
        .expect("the released module wrote the v2 job through the host callback");
    // Module ports cannot set host-owned record metadata, so the migrated
    // job carries its original creation time as the body field `created_ms`
    // (the backend stamps `meta.created_ms` with the write time).
    assert_eq!(migrated.doc["created_ms"], 1772600767123_i64);
    assert!(
        migrated.meta.created_ms > 0,
        "the backend owns record metadata"
    );
    for (collection, id) in [
        ("cron_jobs", "legacy-migration-proof"),
        ("cron_runs", "00000000000000000007"),
        ("cron_counters", "runs"),
    ] {
        assert!(
            docs.get(collection, id).await.unwrap().is_none(),
            "the v25 module retired migrated legacy record {collection}/{id}"
        );
    }
    for progress in [
        "cron/cron.jobs/jobs_v2",
        "cron/cron.runs/runs_v2",
        "cron/cron.admission/admission_v2",
        "cron/counters_import",
    ] {
        assert!(
            docs.get("tsd.migrations", progress)
                .await
                .unwrap()
                .is_some(),
            "the module persisted migration progress {progress}"
        );
    }

    let module_list = post_json_rpc(&rpc_base, 80_003, "openhuman.modules_list", json!({})).await;
    let module_list = peel_logs_envelope(assert_no_jsonrpc_error(&module_list, "modules_list"));
    let tinyflows = module_list["modules"]
        .as_array()
        .and_then(|modules| modules.iter().find(|module| module["id"] == "tinyflows"))
        .unwrap_or_else(|| {
            panic!("TinyFlows is listed after the released cdylib loaded: {module_list}")
        });
    let pinned = openhuman_core::modules::registry::find("tinyflows")
        .expect("TinyFlows is registered in the module asset registry");
    assert_eq!(
        tinyflows["version"], pinned.version,
        "the loaded TinyFlows must be the registry-pinned release"
    );

    let name = format!("published-manual-{}", uuid::Uuid::new_v4().simple());
    let job = openhuman_core::cron::system_jobs::ensure_system_job(
        &config,
        &name,
        openhuman_core::cron::Schedule::Every {
            every_ms: 3_600_000,
        },
    )
    .expect("seed a same-scope manual-worker job");
    let (started_tx, started_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let release_rx = Arc::new(Mutex::new(Some(release_rx)));
    let _handler = system_job_handlers::register(
        &name,
        Arc::new(move |_| {
            let started = started_tx.lock().unwrap().take();
            let release = release_rx.lock().unwrap().take();
            Box::pin(async move {
                if let Some(started) = started {
                    let _ = started.send(());
                }
                if let Some(release) = release {
                    release.await.map_err(|error| error.to_string())?;
                }
                Ok(())
            })
        }),
    );
    struct CronEvents {
        job_id: String,
        tx: mpsc::UnboundedSender<(&'static str, bool)>,
    }
    #[async_trait::async_trait]
    impl EventHandler<DomainEvent> for CronEvents {
        fn name(&self) -> &str {
            "test::json_rpc_e2e::published_cron_manual_lifecycle"
        }
        async fn handle(&self, event: &DomainEvent) {
            let event = match event {
                DomainEvent::CronJobTriggered { job_id, .. } if job_id == &self.job_id => {
                    Some(("triggered", true))
                }
                DomainEvent::CronJobCompleted {
                    job_id, success, ..
                } if job_id == &self.job_id => Some(("completed", *success)),
                _ => None,
            };
            if let Some(event) = event {
                let _ = self.tx.send(event);
            }
        }
    }
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let subscription = openhuman_core::core::bus::BUS
        .subscribe(Arc::new(CronEvents {
            job_id: job.id.clone(),
            tx: events_tx,
        }))
        .expect("subscribe to the production Cron domain events");

    let caller_rpc_base = rpc_base.clone();
    let job_id = job.id.clone();
    let caller = tokio::spawn(async move {
        post_json_rpc(
            &caller_rpc_base,
            80_004,
            "openhuman.cron_run",
            json!({ "job_id": job_id }),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(10), started_rx)
        .await
        .expect("the published module admitted the run and entered the host callback")
        .expect("manual host handler start signal remains open");
    caller.abort();
    match caller.await {
        Err(error) => assert!(
            error.is_cancelled(),
            "caller task failed unexpectedly: {error}"
        ),
        Ok(response) => {
            let queued_ack = peel_logs_envelope(assert_no_jsonrpc_error(
                &response,
                "cron_run may acknowledge before the caller aborts",
            ));
            assert_eq!(queued_ack["job_id"], job.id);
            assert_eq!(queued_ack["status"], "queued");
        }
    }
    let queued_response = post_json_rpc(
        &rpc_base,
        80_006,
        "openhuman.cron_runs",
        json!({ "job_id": job.id }),
    )
    .await;
    let queued_history = peel_logs_envelope(assert_no_jsonrpc_error(
        &queued_response,
        "queued cron_runs while the system handler is held",
    ));
    let queued_run = queued_history
        .as_array()
        .and_then(|runs| runs.first())
        .unwrap_or_else(|| panic!("the admitted run has its queued history: {queued_history}"));
    assert_eq!(queued_run["id"], 8, "v1 next=8 seeds the queue slot");
    assert_eq!(queued_run["status"], "queued");
    let invocation_id = queued_run["run_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .expect("the queued run carries the invocation identity")
        .to_string();
    let _ = release_tx.send(());

    let mut seen_events = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while seen_events.len() < 2 {
            seen_events.push(
                events_rx
                    .recv()
                    .await
                    .expect("Cron event subscription remains open"),
            );
        }
    })
    .await
    .expect("the retained native worker emits trigger and terminal completion");
    assert_eq!(seen_events[0].0, "triggered");
    assert_eq!(seen_events[1], ("completed", true));

    let mut manual_history = serde_json::Value::Null;
    for _ in 0..100 {
        let runs = post_json_rpc(
            &rpc_base,
            80_005,
            "openhuman.cron_runs",
            json!({ "job_id": job.id }),
        )
        .await;
        manual_history = peel_logs_envelope(assert_no_jsonrpc_error(&runs, "cron_runs")).clone();
        if manual_history.to_string().contains("system job")
            && manual_history.to_string().contains("\"status\":\"ok\"")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let terminal_run = manual_history
        .as_array()
        .and_then(|runs| runs.first())
        .unwrap_or_else(|| {
            panic!("the released worker committed terminal history: {manual_history}")
        });
    assert!(
        terminal_run["output"]
            .as_str()
            .is_some_and(|output| output.contains("system job"))
            && terminal_run["status"] == "ok",
        "the released worker committed its terminal history after the caller disconnected: {manual_history}"
    );
    assert!(
        terminal_run["id"] == 9,
        "the terminal record follows queued slot 8 from the imported counter: {manual_history}"
    );
    assert!(
        terminal_run["run_id"] == invocation_id,
        "the terminal record preserves the queued invocation identity: {manual_history}"
    );
    drop(subscription);

    let (scope_started_tx, mut scope_started_rx) = mpsc::unbounded_channel();
    let scope_started_tx = Arc::new(scope_started_tx);
    let scope_release = Arc::new(Semaphore::new(0));
    let handler_started = Arc::clone(&scope_started_tx);
    let handler_release = Arc::clone(&scope_release);
    let scoped_name = format!("published-scope-{}", uuid::Uuid::new_v4().simple());
    let _scope_handler = system_job_handlers::register(
        &scoped_name,
        Arc::new(move |_| {
            let started = Arc::clone(&handler_started);
            let release = Arc::clone(&handler_release);
            Box::pin(async move {
                let _ = started.send(());
                release
                    .acquire()
                    .await
                    .map_err(|error| error.to_string())?
                    .forget();
                Ok(())
            })
        }),
    );
    let agent_context = default_context.derive_with(
        ContextOverlay::new(
            config.clone(),
            DomainSet::full(),
            openhuman_core::tools::toolpacks::ToolGroups::none(),
        )
        .session_agent("published-cron-live-agent"),
    );
    let operator_scope = CoreContext::scope(Arc::clone(&default_context), async {
        openhuman_core::storage::current_scope().unwrap()
    })
    .await;
    let agent_scope = CoreContext::scope(Arc::clone(&agent_context), async {
        openhuman_core::storage::current_scope().unwrap()
    })
    .await;
    assert_ne!(
        operator_scope, agent_scope,
        "the live-agent invocation uses a distinct document scope"
    );
    let first_job = CoreContext::scope(Arc::clone(&default_context), async {
        openhuman_core::cron::system_jobs::ensure_system_job(
            &config,
            &scoped_name,
            openhuman_core::cron::Schedule::Every {
                every_ms: 3_600_000,
            },
        )
    })
    .await
    .unwrap();
    let second_job = CoreContext::scope(Arc::clone(&agent_context), async {
        openhuman_core::cron::system_jobs::ensure_system_job(
            &config,
            &scoped_name,
            openhuman_core::cron::Schedule::Every {
                every_ms: 3_600_000,
            },
        )
    })
    .await
    .unwrap();
    assert_eq!(first_job.id, second_job.id);

    let first_config = config.clone();
    let first_job_id = first_job.id.clone();
    let first_context = Arc::clone(&default_context);
    let first = tokio::spawn(async move {
        let dispatch = DispatchCtx::new(Arc::clone(&first_context));
        CoreContext::scope(first_context, async move {
            openhuman_core::cron::ops::run_job_now(&dispatch, &first_config, &first_job_id).await
        })
        .await
    });
    tokio::time::timeout(Duration::from_secs(10), scope_started_rx.recv())
        .await
        .expect("the first document scope starts its manual worker")
        .expect("scope handler channel remains open");
    first.abort();
    assert!(
        first.await.unwrap_err().is_cancelled(),
        "cancel the core's synchronous Run Now waiter while its module worker remains active"
    );

    let duplicate_context = Arc::clone(&default_context);
    let duplicate_job_id = first_job.id.clone();
    let duplicate_config = config.clone();
    let duplicate = CoreContext::scope(Arc::clone(&duplicate_context), async move {
        openhuman_core::cron::ops::run_job_now(
            &DispatchCtx::new(Arc::clone(&duplicate_context)),
            &duplicate_config,
            &duplicate_job_id,
        )
        .await
    })
    .await;
    let refusal = match duplicate {
        Err(error) => error,
        Ok((false, message)) => message,
        Ok((true, message)) => panic!("same-scope active run unexpectedly succeeded: {message}"),
    };
    assert!(
        refusal.contains("AlreadyRunning"),
        "the same scope refuses the active job: {refusal}"
    );

    let second_config = config.clone();
    let second_job_id = second_job.id.clone();
    let second_context = Arc::clone(&agent_context);
    let mut second = tokio::spawn(async move {
        let dispatch = DispatchCtx::new(Arc::clone(&second_context));
        CoreContext::scope(second_context, async move {
            openhuman_core::cron::ops::run_job_now(&dispatch, &second_config, &second_job_id).await
        })
        .await
    });
    tokio::select! {
        result = &mut second => panic!(
            "a distinct document scope refused the same job id before its callback: {result:?}"
        ),
        started = tokio::time::timeout(Duration::from_secs(10), scope_started_rx.recv()) => {
            started
                .expect("a distinct document scope independently starts the same job id")
                .expect("scope handler channel remains open");
        }
    }
    scope_release.add_permits(2);
    assert!(second.await.unwrap().unwrap().0);
    let (operator_runs, agent_runs) = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let operator_runs = CoreContext::scope(Arc::clone(&default_context), async {
                openhuman_core::cron::list_runs(&config, &first_job.id, 10).unwrap()
            })
            .await;
            let agent_runs = CoreContext::scope(Arc::clone(&agent_context), async {
                openhuman_core::cron::list_runs(&config, &second_job.id, 10).unwrap()
            })
            .await;
            if operator_runs.first().is_some_and(|run| run.status == "ok")
                && agent_runs.first().is_some_and(|run| run.status == "ok")
            {
                break (operator_runs, agent_runs);
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("both scoped workers commit terminal run history after the first caller cancels");
    assert_eq!(operator_runs.len(), 1);
    assert_eq!(agent_runs.len(), 1);
    let operator_run_id = operator_runs[0]
        .run_id
        .as_deref()
        .expect("the operator run stores its invocation identity");
    let agent_run_id = agent_runs[0]
        .run_id
        .as_deref()
        .expect("the agent-scope run stores its invocation identity");
    assert_ne!(operator_run_id, agent_run_id);

    mock_join.abort();
    rpc_join.abort();
}

#[test]
fn json_rpc_cron_origin_delivery_lands_in_the_asking_thread() {
    run_json_rpc_e2e_on_agent_stack(
        "json_rpc_cron_origin_delivery_lands_in_the_asking_thread",
        json_rpc_cron_origin_delivery_lands_in_the_asking_thread_inner,
    );
}

/// A cron job created from a web thread delivers each run's reply into that
/// thread as exactly one assistant row (id `agent:cron:<job>:<run>`), creates no
/// other thread, and a second run adds one more row.
async fn json_rpc_cron_origin_delivery_lands_in_the_asking_thread_inner() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    clear_forced_chat_completions();

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{mock_addr}");
    write_min_config(&openhuman_home, &mock_origin);
    write_min_config(&openhuman_home.join("users").join("e2e-user"), &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let store = post_json_rpc(
        &rpc_base,
        1,
        "openhuman.auth_store_session",
        json!({ "token": "e2e-test-jwt", "user_id": "e2e-user" }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session");

    // The asking thread, with one real web-chat turn so it has an agent transcript.
    let create = post_json_rpc(&rpc_base, 2, "openhuman.threads_create_new", json!({})).await;
    let thread_id = assert_no_jsonrpc_error(&create, "threads_create_new")["data"]["id"]
        .as_str()
        .expect("thread id")
        .to_string();
    let events_url = format!("{rpc_base}/events?client_id=cron-e2e");
    let sse_task = tokio::spawn(async move { read_terminal_web_chat_event(&events_url).await });
    let chat = post_json_rpc(
        &rpc_base,
        3,
        "openhuman.channel_web_chat",
        json!({
            "client_id": "cron-e2e",
            "thread_id": thread_id,
            "message": "remind me to drink water",
            "model_override": "e2e-mock-model",
        }),
    )
    .await;
    assert_no_jsonrpc_error(&chat, "channel_web_chat");
    sse_task.await.expect("chat turn finishes");

    let list_threads = |id: i64| {
        let rpc_base = rpc_base.clone();
        async move {
            let list = post_json_rpc(&rpc_base, id, "openhuman.threads_list", json!({})).await;
            assert_no_jsonrpc_error(&list, "threads_list")["data"]["threads"]
                .as_array()
                .expect("threads array")
                .len()
        }
    };
    let threads_before = list_threads(4).await;

    // An agent job bound to that thread.
    let add = post_json_rpc(
        &rpc_base,
        5,
        "openhuman.cron_add",
        json!({
            "name": "water",
            "schedule": { "kind": "every", "every_ms": 3_600_000 },
            "job_type": "agent",
            "prompt": "Remind the user to drink water.",
            "session_target": "current",
            "delivery": { "mode": "origin" },
            "origin": { "kind": "web", "thread_id": thread_id },
        }),
    )
    .await;
    let add_result = assert_no_jsonrpc_error(&add, "cron_add");
    let job_id = add_result["result"]["id"]
        .as_str()
        .or_else(|| add_result["id"].as_str())
        .unwrap_or_else(|| panic!("job id missing: {add_result}"))
        .to_string();

    let mut delivered_ids: Vec<String> = Vec::new();
    for round in 0..2i64 {
        // Gate on the cron prompt: an unconditional FIFO entry can be taken by
        // any other chat request in the process (a background call after the
        // first turn), which delivered the mock default instead of this reply.
        push_forced_chat_completion_when(
            "Remind the user to drink water.",
            forced_text_completion(&format!("Drink water! #{round}")),
        );
        let run = post_json_rpc(
            &rpc_base,
            10 + round,
            "openhuman.cron_run",
            json!({ "job_id": job_id }),
        )
        .await;
        assert_no_jsonrpc_error(&run, "cron_run");

        // Wait for the delivered run to be recorded.
        let mut ids = Vec::new();
        for _ in 0..100 {
            let runs = post_json_rpc(
                &rpc_base,
                20 + round,
                "openhuman.cron_runs",
                json!({ "job_id": job_id }),
            )
            .await;
            let body = assert_no_jsonrpc_error(&runs, "cron_runs").to_string();
            let done = body.contains("\"delivery_status\":\"delivered\"");
            let msgs = post_json_rpc(
                &rpc_base,
                30 + round,
                "openhuman.threads_messages_list",
                json!({ "thread_id": thread_id }),
            )
            .await;
            ids = assert_no_jsonrpc_error(&msgs, "threads_messages_list")["data"]["messages"]
                .as_array()
                .expect("messages")
                .iter()
                .filter_map(|m| m["id"].as_str().map(str::to_string))
                .filter(|id| id.starts_with(&format!("agent:cron:{job_id}:")))
                .collect();
            if done && ids.len() == (round as usize) + 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(
            ids.len(),
            (round as usize) + 1,
            "exactly one cron reply row per run in the origin thread: {ids:?}"
        );
        delivered_ids = ids;
    }
    assert_eq!(delivered_ids.len(), 2);
    assert_ne!(
        delivered_ids[0], delivered_ids[1],
        "each run has its own row"
    );

    assert_eq!(
        list_threads(40).await,
        threads_before,
        "delivery must not create another thread"
    );

    // The next turn sees the reminder: the agent transcript holds it.
    let transcript = post_json_rpc(
        &rpc_base,
        41,
        "openhuman.threads_transcript_get",
        json!({ "thread_id": thread_id }),
    )
    .await;
    let transcript_body =
        assert_no_jsonrpc_error(&transcript, "threads_transcript_get").to_string();
    assert!(
        transcript_body.contains("Drink water! #0") && transcript_body.contains("Drink water! #1"),
        "transcript must contain both delivered replies: {transcript_body}"
    );

    mock_join.abort();
    rpc_join.abort();
}

#[test]
fn json_rpc_cron_add_run_list_round_trip_over_the_tinyflows_module() {
    run_json_rpc_e2e_on_agent_stack(
        "json_rpc_cron_add_run_list_round_trip_over_the_tinyflows_module",
        json_rpc_cron_add_run_list_round_trip_over_the_tinyflows_module_inner,
    );
}

/// The cron surface end to end over JSON-RPC with the scheduler runtime, job
/// store and schedule maths in the `tinyflows` module: an agent job is added,
/// listed, run now (the module's `ExecuteJob` calling back into the host), its
/// run is recorded, and removing it empties the list.
async fn json_rpc_cron_add_run_list_round_trip_over_the_tinyflows_module_inner() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    clear_forced_chat_completions();

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{mock_addr}");
    write_min_config(&openhuman_home, &mock_origin);
    write_min_config(&openhuman_home.join("users").join("e2e-user"), &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let store = post_json_rpc(
        &rpc_base,
        1,
        "openhuman.auth_store_session",
        json!({ "token": "e2e-test-jwt", "user_id": "e2e-user" }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session");

    // The workspace may hold the host's own system jobs; ours is not there yet.
    let empty = post_json_rpc(&rpc_base, 2, "openhuman.cron_list", json!({})).await;
    assert!(
        !assert_no_jsonrpc_error(&empty, "cron_list")
            .to_string()
            .contains("roundtrip"),
        "the job is not there before it is added"
    );

    let add = post_json_rpc(
        &rpc_base,
        3,
        "openhuman.cron_add",
        json!({
            "name": "roundtrip",
            "schedule": { "kind": "every", "every_ms": 3_600_000 },
            "job_type": "agent",
            "prompt": "Say the round trip phrase.",
            "delivery": { "mode": "none" },
        }),
    )
    .await;
    let add_result = assert_no_jsonrpc_error(&add, "cron_add");
    let job_id = add_result["result"]["id"]
        .as_str()
        .or_else(|| add_result["id"].as_str())
        .unwrap_or_else(|| panic!("job id missing: {add_result}"))
        .to_string();

    let listed = post_json_rpc(&rpc_base, 4, "openhuman.cron_list", json!({})).await;
    let listed_body = assert_no_jsonrpc_error(&listed, "cron_list").to_string();
    assert!(
        listed_body.contains(&job_id) && listed_body.contains("roundtrip"),
        "the added job is listed: {listed_body}"
    );

    push_forced_chat_completion_when(
        "Say the round trip phrase.",
        forced_text_completion("round trip phrase"),
    );
    let run = post_json_rpc(
        &rpc_base,
        5,
        "openhuman.cron_run",
        json!({ "job_id": job_id }),
    )
    .await;
    assert_no_jsonrpc_error(&run, "cron_run");

    let mut recorded = String::new();
    for _ in 0..200 {
        let runs = post_json_rpc(
            &rpc_base,
            6,
            "openhuman.cron_runs",
            json!({ "job_id": job_id }),
        )
        .await;
        recorded = assert_no_jsonrpc_error(&runs, "cron_runs").to_string();
        if recorded.contains("\"status\":\"ok\"") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        recorded.contains("\"status\":\"ok\"") && recorded.contains("round trip phrase"),
        "the run is recorded with the agent's output: {recorded}"
    );

    let remove = post_json_rpc(
        &rpc_base,
        7,
        "openhuman.cron_remove",
        json!({ "job_id": job_id }),
    )
    .await;
    assert_no_jsonrpc_error(&remove, "cron_remove");
    let after = post_json_rpc(&rpc_base, 8, "openhuman.cron_list", json!({})).await;
    assert!(
        !assert_no_jsonrpc_error(&after, "cron_list")
            .to_string()
            .contains(&job_id),
        "the removed job is gone"
    );

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_retired_workflow_run_methods_are_not_found() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_url_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_url_guard = EnvVarGuard::unset("OPENHUMAN_API_URL");

    let (api_addr, api_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let api_origin = format!("http://{api_addr}");
    write_min_config(openhuman_home.as_path(), &api_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    for (id, method) in [
        (9141, "openhuman.workflow_run_list_definitions"),
        (9142, "openhuman.workflow_run_start"),
        (9143, "openhuman.workflow_run_list"),
        (9144, "openhuman.workflow_run_get"),
    ] {
        let response = post_json_rpc(&rpc_base, id, method, json!({})).await;
        assert_unknown_method(&response, method);
    }

    rpc_join.abort();
    api_join.abort();
}

/// Full lifecycle over JSON-RPC for the `workflows` namespace:
/// create → list → read → phase → uninstall. Workflows are scaffolded under
/// the user-scope root (`$HOME/.openhuman/workflows/<slug>/`), which the temp
/// `HOME` isolates per-test.
#[tokio::test]
async fn json_rpc_workflows_lifecycle_round_trip() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_url_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_url_guard = EnvVarGuard::unset("OPENHUMAN_API_URL");

    let (api_addr, api_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let api_origin = format!("http://{api_addr}");
    write_min_config(openhuman_home.as_path(), &api_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    // 1. Create a user-scope workflow.
    let create = post_json_rpc(
        &rpc_base,
        9201,
        "openhuman.skills_create",
        json!({
            "name": "Bug Triage",
            "description": "How to handle an incoming bug report",
            "when_to_use": "a user reports a bug or something is broken",
        }),
    )
    .await;
    let create_result = assert_no_jsonrpc_error(&create, "skills_create");
    let wf = create_result
        .get("workflow")
        .expect("workflow in create result");
    // `id` is the on-disk slug (WorkflowSummary maps dir_name → id). The
    // persisted frontmatter `name` is the slug too (create slugifies it).
    assert_eq!(wf.get("id").and_then(Value::as_str), Some("bug-triage"));
    assert_eq!(wf.get("name").and_then(Value::as_str), Some("bug-triage"));

    // 2. List reflects the new workflow.
    let list = post_json_rpc(&rpc_base, 9202, "openhuman.skills_list", json!({})).await;
    let list_result = assert_no_jsonrpc_error(&list, "skills_list");
    let workflows = list_result
        .get("workflows")
        .and_then(Value::as_array)
        .expect("workflows array in list result");
    assert_eq!(workflows.len(), 1, "exactly one workflow after create");
    assert_eq!(
        workflows[0].get("id").and_then(Value::as_str),
        Some("bug-triage")
    );
    // when_to_use is surfaced via `skills_describe` (step 3), not the list summary.

    // 3. Describe returns the workflow's display name, when-to-use, and inputs.
    let describe = post_json_rpc(
        &rpc_base,
        9203,
        "openhuman.skills_describe",
        json!({ "workflow_id": "bug-triage" }),
    )
    .await;
    let describe_result = assert_no_jsonrpc_error(&describe, "skills_describe");
    // Display name isn't separately preserved today — create slugifies it, so
    // describe reflects the slug. (Preserving the human display name is a known
    // minor gap, tracked separately.)
    assert_eq!(
        describe_result.get("display_name").and_then(Value::as_str),
        Some("bug-triage")
    );
    assert_eq!(
        describe_result.get("when_to_use").and_then(Value::as_str),
        Some("a user reports a bug or something is broken")
    );
    assert!(
        describe_result
            .get("inputs")
            .and_then(Value::as_array)
            .is_some(),
        "describe returns an inputs array (empty for a bare workflow)"
    );

    // 4. Uninstall removes it; the list is empty again.
    let uninstall = post_json_rpc(
        &rpc_base,
        9205,
        "openhuman.skills_uninstall",
        json!({ "name": "bug-triage" }),
    )
    .await;
    let uninstall_result = assert_no_jsonrpc_error(&uninstall, "skills_uninstall");
    assert_eq!(
        uninstall_result.get("name").and_then(Value::as_str),
        Some("bug-triage"),
        "uninstall echoes the slug it removed"
    );

    let after = post_json_rpc(&rpc_base, 9206, "openhuman.skills_list", json!({})).await;
    let after_result = assert_no_jsonrpc_error(&after, "skills_list");
    assert!(
        after_result
            .get("workflows")
            .and_then(Value::as_array)
            .expect("workflows array")
            .is_empty(),
        "no workflows after uninstall"
    );

    api_join.abort();
    rpc_join.abort();
}

// ── openhuman.flows_* (tinyflows) JSON-RPC E2E — PHASE 1e (G5) ─────────────
//
// These exercise the tinyflows `flows_*` controller surface end-to-end over
// HTTP JSON-RPC (distinct from the legacy markdown `workflows_*` namespace
// covered above). Flows persist to a SQLite store + durable checkpoint under
// the temp `$HOME/.openhuman` workspace, so each test isolates `HOME`.
//
// All flows ops wrap their value with a log line, so `into_cli_compatible_json`
// emits `{ "result": <value>, "logs": [...] }` — peel that envelope with
// `peel_logs_envelope` before asserting on the inner payload.

/// Shared boot for a flows E2E: isolates `HOME`, seeds a minimal config against
/// a mock upstream, and stands up the core HTTP router. Returns the rpc base
/// URL plus the join handles + tempdir the caller must keep alive/abort.
#[cfg(feature = "flows")]
async fn boot_flows_rpc_env() -> (
    String,
    tempfile::TempDir,
    tokio::task::JoinHandle<std::io::Result<()>>,
    tokio::task::JoinHandle<std::io::Result<()>>,
    Vec<EnvVarGuard>,
) {
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
    ];

    let (api_addr, api_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let api_origin = format!("http://{api_addr}");
    write_min_config(openhuman_home.as_path(), &api_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");

    (rpc_base, tmp, api_join, rpc_join, guards)
}

/// The smallest valid graph with a human-in-the-loop approval gate:
/// `trigger → gate(requires_approval) → downstream`. A run pauses at `gate`;
/// approving it via `flows_resume` runs `downstream`.
#[cfg(feature = "flows")]
fn approval_gated_graph_json() -> Value {
    json!({
        "name": "approval-gated",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "gate", "kind": "output_parser", "name": "Gate", "config": { "requires_approval": true } },
            { "id": "downstream", "kind": "output_parser", "name": "Downstream" }
        ],
        "edges": [
            { "from_node": "t", "to_node": "gate" },
            { "from_node": "gate", "to_node": "downstream" }
        ]
    })
}

/// Full flows lifecycle over JSON-RPC:
/// create → run (park on approval) → inspect steps via get_run/list_runs →
/// resume (approve) to completion → run again → cancel_run the parked run →
/// delete. Note: there is no dedicated `flows_cancel` for an in-flight run;
/// cancel operates on a run id (checkpoint thread id), so we cancel a *fresh*
/// parked run rather than the already-completed one (cancelling a terminal run
/// is an error).
#[cfg(feature = "flows")]
#[tokio::test]
async fn json_rpc_flows_lifecycle_round_trip() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let (rpc_base, _tmp, api_join, rpc_join, _guards) = boot_flows_rpc_env().await;

    // 1. Create an approval-gated flow.
    let create = post_json_rpc(
        &rpc_base,
        9301,
        "openhuman.flows_create",
        json!({ "name": "Gated Demo", "graph": approval_gated_graph_json() }),
    )
    .await;
    let flow = peel_logs_envelope(assert_no_jsonrpc_error(&create, "flows_create"));
    let flow_id = flow
        .get("id")
        .and_then(Value::as_str)
        .expect("flow id in create result")
        .to_string();
    assert_eq!(flow.get("name").and_then(Value::as_str), Some("Gated Demo"));
    assert_eq!(flow.get("enabled").and_then(Value::as_bool), Some(true));

    // 2. List reflects exactly the new flow.
    let list = post_json_rpc(&rpc_base, 9302, "openhuman.flows_list", json!({})).await;
    let flows = peel_logs_envelope(assert_no_jsonrpc_error(&list, "flows_list"))
        .as_array()
        .expect("flows array");
    assert_eq!(flows.len(), 1, "exactly one flow after create");
    assert_eq!(
        flows[0].get("id").and_then(Value::as_str),
        Some(flow_id.as_str())
    );

    // 3. Run — pauses at the approval gate. Capture the thread id.
    let run = post_json_rpc(
        &rpc_base,
        9303,
        "openhuman.flows_run",
        json!({ "id": flow_id, "input": { "x": 1 } }),
    )
    .await;
    let run_out = peel_logs_envelope(assert_no_jsonrpc_error(&run, "flows_run"));
    let thread_id = run_out
        .get("thread_id")
        .and_then(Value::as_str)
        .expect("thread_id in run result")
        .to_string();
    let pending = run_out
        .get("pending_approvals")
        .and_then(Value::as_array)
        .expect("pending_approvals array");
    assert!(
        pending.iter().any(|v| v == "gate"),
        "run must park on the `gate` approval, got: {pending:?}"
    );
    // Downstream must NOT have run while the gate is pending.
    assert!(run_out["output"]["nodes"]["downstream"].is_null());

    // 4. Inspect the run via get_run — status pending_approval, steps recorded.
    let get_run = post_json_rpc(
        &rpc_base,
        9304,
        "openhuman.flows_get_run",
        json!({ "run_id": thread_id }),
    )
    .await;
    let run_row = peel_logs_envelope(assert_no_jsonrpc_error(&get_run, "flows_get_run"));
    assert_eq!(
        run_row.get("status").and_then(Value::as_str),
        Some("pending_approval")
    );
    assert_eq!(
        run_row.get("thread_id").and_then(Value::as_str),
        Some(thread_id.as_str())
    );
    assert!(
        run_row.get("steps").and_then(Value::as_array).is_some(),
        "get_run returns a steps array"
    );

    // 5. list_runs surfaces the same single run.
    let list_runs = post_json_rpc(
        &rpc_base,
        9305,
        "openhuman.flows_list_runs",
        json!({ "id": flow_id }),
    )
    .await;
    let runs = peel_logs_envelope(assert_no_jsonrpc_error(&list_runs, "flows_list_runs"))
        .as_array()
        .expect("runs array");
    assert_eq!(runs.len(), 1, "exactly one run after a single flows_run");
    assert_eq!(
        runs[0].get("id").and_then(Value::as_str),
        Some(thread_id.as_str())
    );

    // 6. Resume with the gate approved — completes and runs downstream.
    let resume = post_json_rpc(
        &rpc_base,
        9306,
        "openhuman.flows_resume",
        json!({ "id": flow_id, "thread_id": thread_id, "approvals": ["gate"] }),
    )
    .await;
    let resume_out = peel_logs_envelope(assert_no_jsonrpc_error(&resume, "flows_resume"));
    assert_eq!(resume_out["pending_approvals"], json!([]));
    assert!(
        !resume_out["output"]["nodes"]["downstream"]["items"].is_null(),
        "downstream should run once the gate is approved via resume"
    );

    // The persisted run row must now be terminal `completed`.
    let get_run2 = post_json_rpc(
        &rpc_base,
        9307,
        "openhuman.flows_get_run",
        json!({ "run_id": thread_id }),
    )
    .await;
    let run_row2 = peel_logs_envelope(assert_no_jsonrpc_error(&get_run2, "flows_get_run"));
    assert_eq!(
        run_row2.get("status").and_then(Value::as_str),
        Some("completed")
    );

    // 7. Run again to get a fresh parked run, then cancel it.
    let run2 = post_json_rpc(
        &rpc_base,
        9308,
        "openhuman.flows_run",
        json!({ "id": flow_id, "input": { "x": 2 } }),
    )
    .await;
    let run2_out = peel_logs_envelope(assert_no_jsonrpc_error(&run2, "flows_run"));
    let thread_id_2 = run2_out
        .get("thread_id")
        .and_then(Value::as_str)
        .expect("thread_id in second run")
        .to_string();

    let cancel = post_json_rpc(
        &rpc_base,
        9309,
        "openhuman.flows_cancel_run",
        json!({ "run_id": thread_id_2 }),
    )
    .await;
    let cancel_out = peel_logs_envelope(assert_no_jsonrpc_error(&cancel, "flows_cancel_run"));
    assert_eq!(
        cancel_out.get("cancelled").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        cancel_out.get("run_id").and_then(Value::as_str),
        Some(thread_id_2.as_str())
    );
    // A parked run has no live task, so cancel settles the row directly.
    assert_eq!(
        cancel_out.get("was_in_flight").and_then(Value::as_bool),
        Some(false)
    );

    // The cancelled run is terminal and no longer resumable.
    let get_run3 = post_json_rpc(
        &rpc_base,
        9310,
        "openhuman.flows_get_run",
        json!({ "run_id": thread_id_2 }),
    )
    .await;
    let run_row3 = peel_logs_envelope(assert_no_jsonrpc_error(&get_run3, "flows_get_run"));
    assert_eq!(
        run_row3.get("status").and_then(Value::as_str),
        Some("cancelled")
    );

    let resume_cancelled = post_json_rpc(
        &rpc_base,
        9311,
        "openhuman.flows_resume",
        json!({ "id": flow_id, "thread_id": thread_id_2, "approvals": ["gate"] }),
    )
    .await;
    assert_jsonrpc_error(&resume_cancelled, "resume of a cancelled run must error");

    // 8. Delete the flow; the list is empty again.
    let delete = post_json_rpc(
        &rpc_base,
        9312,
        "openhuman.flows_delete",
        json!({ "id": flow_id }),
    )
    .await;
    let del_out = peel_logs_envelope(assert_no_jsonrpc_error(&delete, "flows_delete"));
    assert_eq!(del_out.get("removed").and_then(Value::as_bool), Some(true));

    let after = post_json_rpc(&rpc_base, 9313, "openhuman.flows_list", json!({})).await;
    assert!(
        peel_logs_envelope(assert_no_jsonrpc_error(&after, "flows_list"))
            .as_array()
            .expect("flows array")
            .is_empty(),
        "no flows after delete"
    );

    api_join.abort();
    rpc_join.abort();
}

/// JSON-RPC E2E for `flows.run_detached` (PR E / F-M1, F-M2): unlike
/// `flows_run`, which blocks the RPC caller until the run reaches a terminal
/// status, `flows_run_detached` must register the run and return
/// `{run_id, status: "running", detached: true}` immediately — the run row
/// must already be readable via `flows_get_run` the instant this call
/// returns, and only *later* (via polling here, `flow:run_progress` /
/// `useFlowRunPoller` in the UI) does it settle to a terminal status. This is
/// the RPC-layer counterpart to the direct-API tests
/// `flows_run_detached_returns_running_run_id_and_inserts_row` /
/// `flows_run_detached_registers_the_run_before_returning_its_id` in
/// `crates/openhuman-core/src/flows/ops_tests.rs` — same contract, exercised through the
/// `openhuman.flows_run_detached` controller (schema + handler wiring), not
/// just the `ops::flows_run_detached` fn directly.
#[cfg(feature = "flows")]
#[tokio::test]
async fn json_rpc_flows_run_detached_returns_before_run_completes() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let (rpc_base, _tmp, api_join, rpc_join, _guards) = boot_flows_rpc_env().await;

    // 1. Create a trivial, immediately-completable flow.
    let create = post_json_rpc(
        &rpc_base,
        9401,
        "openhuman.flows_create",
        json!({
            "name": "Detached Demo",
            "graph": {
                "name": "Detached Demo",
                "nodes": [{ "id": "t", "kind": "trigger", "name": "Trigger" }],
                "edges": []
            }
        }),
    )
    .await;
    let flow = peel_logs_envelope(assert_no_jsonrpc_error(&create, "flows_create"));
    let flow_id = flow
        .get("id")
        .and_then(Value::as_str)
        .expect("flow id in create result")
        .to_string();

    // 2. `flows_run_detached` must hand back a non-terminal "running" payload
    //    right away, not the completed-run payload `flows_run` returns.
    let run = post_json_rpc(
        &rpc_base,
        9402,
        "openhuman.flows_run_detached",
        json!({ "id": flow_id }),
    )
    .await;
    let run_out = peel_logs_envelope(assert_no_jsonrpc_error(&run, "flows_run_detached"));
    assert_eq!(
        run_out.get("status").and_then(Value::as_str),
        Some("running"),
        "run_detached must report 'running' immediately, not await a terminal status: {run_out:?}"
    );
    assert_eq!(run_out.get("detached").and_then(Value::as_bool), Some(true));
    assert_eq!(
        run_out.get("flow_id").and_then(Value::as_str),
        Some(flow_id.as_str())
    );
    let run_id = run_out
        .get("run_id")
        .and_then(Value::as_str)
        .expect("run_id in run_detached result")
        .to_string();

    // 3. The run row must already exist — registered + inserted synchronously
    //    before `flows_run_detached` returned, not from inside the spawned
    //    background task (which may not be polled for a while).
    let get_run = post_json_rpc(
        &rpc_base,
        9403,
        "openhuman.flows_get_run",
        json!({ "run_id": run_id }),
    )
    .await;
    let run_row = peel_logs_envelope(assert_no_jsonrpc_error(&get_run, "flows_get_run"));
    assert_eq!(
        run_row.get("id").and_then(Value::as_str),
        Some(run_id.as_str())
    );

    // 4. The background task eventually settles the row to a terminal status
    //    on its own — this is the completion signal the UI's poller /
    //    `flow:run_progress` subscription relies on instead of the RPC return.
    let mut terminal_status: Option<String> = None;
    for _ in 0..50 {
        let get_run = post_json_rpc(
            &rpc_base,
            9404,
            "openhuman.flows_get_run",
            json!({ "run_id": run_id }),
        )
        .await;
        let run_row = peel_logs_envelope(assert_no_jsonrpc_error(&get_run, "flows_get_run"));
        let status = run_row
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if status != "running" {
            terminal_status = Some(status);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        matches!(
            terminal_status.as_deref(),
            Some("completed") | Some("completed_with_warnings")
        ),
        "the detached run must settle to a terminal status on its own, got: {terminal_status:?}"
    );

    api_join.abort();
    rpc_join.abort();
}

/// JSON-RPC regression for the Rule 2 compound-bypass fix (C1): `flows_update`
/// must re-derive `require_approval` from the *effective* graph, not trust the
/// caller's raw toggle. This is the RPC-layer counterpart to the direct-API
/// tests `flows_update_forces_require_approval_when_adding_side_effect_nodes`
/// / `flows_update_does_not_force_require_approval_on_readonly_graph` in
/// `crates/openhuman-core/src/flows/ops_tests.rs` — same rule, exercised through the
/// `openhuman.flows_update` controller (schema + handler wiring), not just
/// the `ops::flows_update` fn directly.
#[cfg(feature = "flows")]
#[tokio::test]
async fn json_rpc_flows_update_forces_require_approval_on_side_effect_graph() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let (rpc_base, _tmp, api_join, rpc_join, _guards) = boot_flows_rpc_env().await;

    // 1. Create a trigger-only (read-only) flow with require_approval: false.
    let create = post_json_rpc(
        &rpc_base,
        9401,
        "openhuman.flows_create",
        json!({
            "name": "rpc-rule2-demo",
            "graph": {
                "name": "trigger-only",
                "nodes": [ { "id": "t", "kind": "trigger", "name": "Trigger" } ],
                "edges": []
            },
            "require_approval": false
        }),
    )
    .await;
    let flow = peel_logs_envelope(assert_no_jsonrpc_error(&create, "flows_create"));
    let flow_id = flow
        .get("id")
        .and_then(Value::as_str)
        .expect("flow id in create result")
        .to_string();
    assert_eq!(
        flow.get("require_approval").and_then(Value::as_bool),
        Some(false),
        "a trigger-only graph must not force require_approval on create"
    );

    // 2. Update to a graph with an outbound Composio tool_call node, still
    // passing require_approval: false — the RPC handler must force it to
    // true rather than trust the caller's toggle.
    let update = post_json_rpc(
        &rpc_base,
        9402,
        "openhuman.flows_update",
        json!({
            "id": flow_id,
            "graph": {
                "name": "with-tool-call",
                "nodes": [
                    { "id": "t", "kind": "trigger", "name": "Trigger" },
                    {
                        "id": "post",
                        "kind": "tool_call",
                        "name": "Post",
                        "config": { "slug": "SLACK_SEND_MESSAGE", "args": { "channel": "general" } }
                    }
                ],
                "edges": [ { "from_node": "t", "to_node": "post" } ]
            },
            "require_approval": false
        }),
    )
    .await;
    let updated = peel_logs_envelope(assert_no_jsonrpc_error(&update, "flows_update"));
    assert_eq!(
        updated.get("require_approval").and_then(Value::as_bool),
        Some(true),
        "flows_update over JSON-RPC must force require_approval=true when the \
         replacement graph adds an outbound side-effect node, even though the \
         caller passed false"
    );

    // 3. The forced value must also be what's persisted, not just what's
    // echoed back in the update response.
    let get = post_json_rpc(
        &rpc_base,
        9403,
        "openhuman.flows_get",
        json!({ "id": flow_id }),
    )
    .await;
    let persisted = peel_logs_envelope(assert_no_jsonrpc_error(&get, "flows_get"));
    assert_eq!(
        persisted.get("require_approval").and_then(Value::as_bool),
        Some(true),
        "the forced require_approval must be persisted, not just returned"
    );

    api_join.abort();
    rpc_join.abort();
}

/// Flow Scout suggestion-lifecycle methods over JSON-RPC (no LLM involved):
/// `flows_list_suggestions` starts empty, and `flows_dismiss_suggestion` /
/// `flows_mark_suggestion_built` on an unknown id resolve cleanly and report
/// `false`. This pins that the four new controllers are registered and dispatch
/// end-to-end (schema + handler wiring), independent of the agent-backed
/// `flows_discover`, which needs a provider.
#[cfg(feature = "flows")]
#[tokio::test]
async fn json_rpc_flows_suggestion_lifecycle_methods_are_wired() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let (rpc_base, _tmp, api_join, rpc_join, _guards) = boot_flows_rpc_env().await;

    // Empty to start.
    let list = post_json_rpc(
        &rpc_base,
        9501,
        "openhuman.flows_list_suggestions",
        json!({}),
    )
    .await;
    assert!(
        peel_logs_envelope(assert_no_jsonrpc_error(&list, "flows_list_suggestions"))
            .as_array()
            .expect("suggestions array")
            .is_empty(),
        "no suggestions before any discovery run"
    );

    // Filtered list also resolves (status param accepted).
    let list_new = post_json_rpc(
        &rpc_base,
        9502,
        "openhuman.flows_list_suggestions",
        json!({ "status": "new" }),
    )
    .await;
    assert_no_jsonrpc_error(&list_new, "flows_list_suggestions(status=new)");

    // Dismiss/mark unknown ids resolve without error and report false.
    let dismiss = post_json_rpc(
        &rpc_base,
        9503,
        "openhuman.flows_dismiss_suggestion",
        json!({ "id": "does-not-exist" }),
    )
    .await;
    let dismiss_out = peel_logs_envelope(assert_no_jsonrpc_error(
        &dismiss,
        "flows_dismiss_suggestion",
    ));
    assert_eq!(
        dismiss_out.get("dismissed").and_then(Value::as_bool),
        Some(false)
    );

    let built = post_json_rpc(
        &rpc_base,
        9504,
        "openhuman.flows_mark_suggestion_built",
        json!({ "id": "does-not-exist" }),
    )
    .await;
    let built_out = peel_logs_envelope(assert_no_jsonrpc_error(
        &built,
        "flows_mark_suggestion_built",
    ));
    assert_eq!(built_out.get("built").and_then(Value::as_bool), Some(false));

    api_join.abort();
    rpc_join.abort();
}

/// Minimal config for the agent-backed flows arc: like `write_min_config` but
/// pins `default_model` to a concrete catalog model so both agent nodes — the
/// **chat**-role drafter and the **reasoning**-role planner — resolve to that
/// pinned managed model, letting the full-arc test assert the pin took.
#[cfg(feature = "flows")]
fn write_flows_tier_config(openhuman_dir: &Path, api_origin: &str) {
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "openrouter/deepseek/deepseek-v4-pro"
default_temperature = 0.7
chat_onboarding_completed = true

[secrets]
encrypt = false

# Keep tool-result content raw: the workflow_builder agent runs the reasoning
# tier with Full TokenJuice compaction, which CCR-compresses a large
# `propose_workflow` result into a `juice_retrieve` reference. `flows_build`
# extracts the proposal from the agent's tool history by JSON-parsing that
# content, so compaction must stay off for the arc to be observable in-test.
[context]
compaction_enabled = false

[autonomy]
auto_approve = ["run_flow"]
"#
    );
    fn write_config_file(config_dir: &Path, cfg: &str) {
        std::fs::create_dir_all(config_dir).expect("mkdir openhuman");
        std::fs::write(config_dir.join("config.toml"), cfg).expect("write config");
    }
    write_config_file(openhuman_dir, &cfg);
    if openhuman_dir
        .file_name()
        .is_some_and(|name| name == std::ffi::OsStr::new(".openhuman"))
    {
        write_config_file(&openhuman_dir.join("users").join("local"), &cfg);
    }
    let _: openhuman_core::config::Config =
        toml::from_str(&cfg).expect("config toml must match Config schema");
}

/// The canonical "Research brief (Opus plans, Sonnet drafts)" demo graph — the
/// same shape as `app/src/lib/flows/templates/opus-sonnet-brief.json`, inlined so
/// the e2e test doesn't depend on reading the frontend template at runtime. A
/// `trigger` feeds a reasoning-tier `planner` (structured `{plan, angle}`) into a
/// chat-tier `drafter` that references `nodes.planner.item.json.plan`, then a
/// `transform` shapes `{topic, plan, draft}`.
#[cfg(feature = "flows")]
fn opus_sonnet_demo_graph() -> Value {
    json!({
        "schema_version": 1,
        "name": "Research brief (Opus plans, Sonnet drafts)",
        "nodes": [
            {
                "id": "trigger",
                "kind": "trigger",
                "name": "Run manually with a topic",
                "config": { "trigger_kind": "manual" }
            },
            {
                "id": "planner",
                "kind": "agent",
                "name": "Plan the brief (reasoning tier)",
                "config": {
                    "model": "hint:reasoning",
                    "prompt": "=\"You are a research lead. Draft a concise research plan (3-5 steps) and pick one distinctive angle for a brief on: \" + (.run.trigger.topic // \"the requested topic\")",
                    "output_parser": {
                        "schema": {
                            "type": "object",
                            "required": ["plan", "angle"],
                            "properties": {
                                "plan": { "type": "string" },
                                "angle": { "type": "string" }
                            }
                        }
                    }
                }
            },
            {
                "id": "drafter",
                "kind": "agent",
                "name": "Draft the brief (chat tier)",
                "config": {
                    "model": "hint:chat",
                    "prompt": "=\"Using the plan and angle below, write a polished research brief (~300 words).\\n\\nPlan:\\n\" + (.nodes.planner.item.json.plan // \"\") + \"\\n\\nAngle:\\n\" + (.nodes.planner.item.json.angle // \"\")"
                }
            },
            {
                "id": "shape",
                "kind": "transform",
                "name": "Shape the result",
                "config": {
                    "set": {
                        "topic": "=run.trigger.topic",
                        "plan": "=nodes.planner.item.json.plan",
                        "draft": "=nodes.drafter.item.text"
                    }
                }
            }
        ],
        "edges": [
            { "from_node": "trigger", "from_port": "main", "to_node": "planner", "to_port": "main" },
            { "from_node": "planner", "from_port": "main", "to_node": "drafter", "to_port": "main" },
            { "from_node": "drafter", "from_port": "main", "to_node": "shape", "to_port": "main" }
        ]
    })
}

/// Full flows arc over JSON-RPC, driven entirely by the scripted-completion FIFO:
///
/// 1. `flows_discover` — the Flow Scout scripts a `suggest_workflows` tool call;
///    the recorded suggestion is asserted via `flows_list_suggestions`.
/// 2. `flows_build` — the workflow_builder scripts a `propose_workflow` tool call
///    carrying the Opus+Sonnet demo graph; the returned proposal is asserted.
/// 3. `flows_create` + `flows_run` — the saved demo graph runs with two scripted
///    plain completions (planner structured JSON, then drafter text). We assert
///    the run completed, the planner's structured plan flowed into the drafter's
///    prompt (data passing), and the two agent nodes routed to the expected
///    pinned managed model.
///
/// Runs on the agent-sized worker stack because the builder/scout turns and the
/// agent-node run drive the full harness (deep async stacks).
#[cfg(feature = "flows")]
#[test]
fn json_rpc_flows_full_arc_discover_build_create_run() {
    const CHILD_ENV: &str = "OPENHUMAN_COPILOT_RUN_FLOW_E2E_CHILD";
    if std::env::var_os(CHILD_ENV).is_none() {
        // ApprovalGate is process-global and one-shot. Run this case in an
        // exact-filtered child so the test's narrow run_flow auto-approval
        // cannot leak into any sibling E2E case in a broad cargo invocation.
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "json_rpc_flows_full_arc_discover_build_create_run",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, "1")
            .output()
            .expect("run the isolated Copilot RunFlow E2E child");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        print!("{stdout}");
        eprint!("{stderr}");
        let child_output = format!("{stdout}{stderr}");
        assert!(
            output.status.success(),
            "isolated Copilot RunFlow E2E child failed:\n{child_output}"
        );
        assert!(
            child_output.contains("running 1 test")
                && child_output
                    .contains("test json_rpc_flows_full_arc_discover_build_create_run ... ok")
                && child_output.contains("1 passed; 0 failed"),
            "child must prove exactly one test ran and passed:\n{child_output}"
        );
        return;
    }
    run_json_rpc_e2e_on_agent_stack(
        "json_rpc_flows_full_arc_discover_build_create_run",
        json_rpc_flows_full_arc_discover_build_create_run_inner,
    );
}

#[cfg(feature = "flows")]
async fn json_rpc_flows_full_arc_discover_build_create_run_inner() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    // Drain the scripted-completion FIFO even if an assertion below panics, so a
    // leftover entry can't bleed into another test sharing this binary.
    let _fifo_guard = ScriptedFifoGuard;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_url_guard = EnvVarGuard::unset("OPENHUMAN_API_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{mock_addr}");
    write_flows_tier_config(&openhuman_home, &mock_origin);
    let user_scoped_dir = openhuman_home.join("users").join("e2e-user");
    write_flows_tier_config(&user_scoped_dir, &mock_origin);

    // This test runs as its own exact-filtered JSON-RPC E2E process. Install
    // only the approval needed for the real run_flow tool call below, so the
    // streaming builder exposes that tool without parking the deterministic
    // test completion for a human decision.
    let mut approval_config = openhuman_core::config::Config {
        workspace_dir: openhuman_home.join("workspace"),
        config_path: openhuman_home.join("config.toml"),
        ..openhuman_core::config::Config::default()
    };
    approval_config
        .autonomy
        .auto_approve
        .push("run_flow".to_string());
    openhuman_core::security::approval::ApprovalGate::init_global(
        approval_config,
        format!("session-{}", uuid::Uuid::new_v4()),
    );

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The managed backend needs an active session JWT to build agent-node /
    // builder / scout completions; the mock validates it via GET /auth/me.
    let store = post_json_rpc(
        &rpc_base,
        71_000,
        "openhuman.auth_store_session",
        json!({ "token": "e2e-test-jwt", "user_id": "e2e-user" }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session");

    // ── 1. flows_discover: the scout scripts a suggest_workflows tool call ──
    clear_forced_chat_completions();
    with_chat_completion_requests(|requests| requests.clear());
    // Turn 1: emit the terminal `suggest_workflows` tool call. Turn 2: a plain
    // reply ends the scout's tool loop. Both gate on "suggest_workflows" (present
    // in the scout's tool schema every turn, absent from every other agent).
    push_forced_chat_completion_when(
        "suggest_workflows",
        forced_tool_call_completion(
            "suggest_workflows",
            json!({
                "suggestions": [{
                    "title": "Weekly research brief",
                    "one_liner": "Plan then draft a research brief on a topic each week.",
                    "rationale": "Your recent threads keep asking for structured research briefs.",
                    "build_prompt": "Build a workflow where a reasoning model plans a research brief and a chat model drafts it."
                }]
            }),
        ),
    );
    push_forced_chat_completion_when(
        "suggest_workflows",
        forced_text_completion("Recorded one workflow suggestion for you."),
    );

    let discover = post_json_rpc(&rpc_base, 71_001, "openhuman.flows_discover", json!({})).await;
    let discovered = peel_logs_envelope(assert_no_jsonrpc_error(&discover, "flows_discover"))
        .as_array()
        .cloned()
        .expect("flows_discover returns a suggestions array");
    assert!(
        discovered
            .iter()
            .any(|s| s.get("title").and_then(Value::as_str) == Some("Weekly research brief")),
        "scripted suggestion should be returned by flows_discover: {discovered:?}"
    );

    // The suggestion must have persisted — assert via the independent list path.
    let listed = post_json_rpc(
        &rpc_base,
        71_002,
        "openhuman.flows_list_suggestions",
        json!({ "status": "new" }),
    )
    .await;
    let listed = peel_logs_envelope(assert_no_jsonrpc_error(&listed, "flows_list_suggestions"))
        .as_array()
        .cloned()
        .expect("flows_list_suggestions returns an array");
    assert!(
        listed
            .iter()
            .any(|s| s.get("title").and_then(Value::as_str) == Some("Weekly research brief")),
        "scripted suggestion should be persisted: {listed:?}"
    );

    // ── 2. flows_build: the builder exercises the Copilot bus and sandbox ──
    clear_forced_chat_completions();
    with_chat_completion_requests(|requests| requests.clear());
    let demo_graph = opus_sonnet_demo_graph();
    let invalid_preflight_graph = json!({
        "nodes": [
            { "id": "trigger", "kind": "trigger", "name": "Manual" },
            { "id": "send", "kind": "tool_call", "name": "Incomplete action",
              "config": { "slug": "GMAIL_ADD_LABEL_TO_EMAIL", "args": { "add_label_ids": ["INBOX"] } } }
        ],
        "edges": [ { "from_node": "trigger", "to_node": "send" } ]
    });
    let sandbox_graph = json!({
        "nodes": [
            { "id": "trigger", "kind": "trigger", "name": "Manual" },
            { "id": "search", "kind": "tool_call", "name": "Mock search",
              "config": { "slug": "oh:web_search", "args": { "query": "sandbox callback probe" } } }
        ],
        "edges": [ { "from_node": "trigger", "to_node": "search" } ]
    });
    // First reject a missing required arg through the preflight-only CapsHost
    // callback. Then accept a valid native tool node using module MockTools,
    // propose the demo graph, and end the turn with a plain reply.
    push_forced_chat_completion_when(
        "dry_run_workflow",
        forced_tool_call_completion(
            "dry_run_workflow",
            json!({ "graph": invalid_preflight_graph }),
        ),
    );
    push_forced_chat_completion_when(
        "message_id",
        forced_tool_call_completion("dry_run_workflow", json!({ "graph": sandbox_graph })),
    );
    push_forced_chat_completion_when(
        "\\\"sandbox\\\": true",
        forced_tool_call_completion(
            "propose_workflow",
            json!({
                "name": "Research brief (Opus plans, Sonnet drafts)",
                "graph": demo_graph.clone()
            }),
        ),
    );
    push_forced_chat_completion_when(
        "propose_workflow",
        forced_text_completion("Here's a research-brief workflow: Opus plans, Sonnet drafts."),
    );

    let build = post_json_rpc(
        &rpc_base,
        71_010,
        "openhuman.flows_build",
        json!({
            "mode": "create",
            "instruction": "Build a research brief workflow where a reasoning model plans and a chat model drafts."
        }),
    )
    .await;
    let build_out = peel_logs_envelope(assert_no_jsonrpc_error(&build, "flows_build"));
    let proposal = build_out
        .get("proposal")
        .filter(|p| !p.is_null())
        .unwrap_or_else(|| panic!("flows_build returns a non-null proposal: {build_out}"));
    assert_eq!(
        proposal.get("type").and_then(Value::as_str),
        Some("workflow_proposal")
    );
    let proposed_ids: Vec<&str> = proposal
        .pointer("/graph/nodes")
        .and_then(Value::as_array)
        .expect("proposal graph has nodes")
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str))
        .collect();
    assert!(
        proposed_ids.contains(&"planner") && proposed_ids.contains(&"drafter"),
        "proposal graph must carry the agent nodes: {proposed_ids:?}"
    );
    let builder_requests = with_chat_completion_requests(|requests| requests.clone());
    assert!(
        builder_requests.iter().any(|request| {
            request
                .get("body")
                .map(Value::to_string)
                .is_some_and(|body| body.contains("\\\"sandbox\\\": true"))
        }),
        "the second builder call must receive the released module's sandbox report: {builder_requests:?}"
    );
    assert!(
        builder_requests.iter().any(|request| {
            request
                .get("body")
                .map(Value::to_string)
                .is_some_and(|body| body.contains("message_id") && body.contains("required arg"))
        }),
        "the invalid sandbox graph must return the required-arg preflight error before any mock call: {builder_requests:?}"
    );

    // ── 3. flows_create + flows_run: the saved graph runs its agent nodes ──
    clear_forced_chat_completions();
    with_chat_completion_requests(|requests| requests.clear());
    with_chat_completion_models(|models| models.clear());

    let create = post_json_rpc(
        &rpc_base,
        71_020,
        "openhuman.flows_create",
        json!({ "name": "Research brief (e2e)", "graph": demo_graph.clone() }),
    )
    .await;
    let flow_id = peel_logs_envelope(assert_no_jsonrpc_error(&create, "flows_create"))
        .get("id")
        .and_then(Value::as_str)
        .expect("flow id from flows_create")
        .to_string();

    // Planner (reasoning tier, structured): return the `{plan, angle}` object.
    // The plan carries a marker so the drafter request can be proven to reference
    // it. Gated on "research lead" — unique to the planner's rendered prompt.
    push_forced_chat_completion_when(
        "research lead",
        forced_text_completion(
            "{\"plan\":\"PLAN_MARKER: interview 3 experts, survey the literature, synthesize findings.\",\"angle\":\"a contrarian systems view\"}",
        ),
    );
    // Drafter (chat tier, plain text). Gated on "polished research brief" — unique
    // to the drafter's rendered prompt.
    push_forced_chat_completion_when(
        "polished research brief",
        forced_text_completion("DRAFT_MARKER: a polished research brief distilled from the plan."),
    );

    let run = post_json_rpc(
        &rpc_base,
        71_021,
        "openhuman.flows_run",
        json!({ "id": flow_id, "input": { "topic": "renewable microgrids" } }),
    )
    .await;
    let run_out = peel_logs_envelope(assert_no_jsonrpc_error(&run, "flows_run"));
    assert!(
        run_out
            .get("pending_approvals")
            .and_then(Value::as_array)
            .is_some_and(|a| a.is_empty()),
        "run should complete with no pending approvals: {run_out}"
    );
    let output_str = run_out
        .get("output")
        .expect("run output present")
        .to_string();
    assert!(
        output_str.contains("PLAN_MARKER") && output_str.contains("DRAFT_MARKER"),
        "run output should carry the planner's plan and the drafter's text: {output_str}"
    );

    // Assert the two agent-node completions routed to the expected managed tiers
    // and that the planner's structured plan flowed into the drafter's prompt.
    let requests = with_chat_completion_requests(|requests| requests.clone());
    let body_of = |r: &Value| r.get("body").map(Value::to_string).unwrap_or_default();
    let planner_req = requests
        .iter()
        .find(|r| body_of(r).contains("research lead"))
        .cloned()
        .expect("planner completion should have been captured");
    assert_eq!(
        planner_req.get("model").and_then(Value::as_str),
        Some("openrouter/deepseek/deepseek-v4-pro"),
        "planner node (reasoning role) must resolve to the pinned managed model"
    );
    let drafter_req = requests
        .iter()
        .find(|r| body_of(r).contains("polished research brief"))
        .cloned()
        .expect("drafter completion should have been captured");
    assert_eq!(
        drafter_req.get("model").and_then(Value::as_str),
        Some("openrouter/deepseek/deepseek-v4-pro"),
        "drafter node (chat role) must resolve to the pinned managed model"
    );
    assert!(
        body_of(&drafter_req).contains("PLAN_MARKER"),
        "drafter prompt must reference the planner's structured plan (graph data passing)"
    );

    // ── 4. Copilot run_flow goes through InvokeTool → RunFlow over TinyBus ──
    // The trigger-only flow finishes without a model call, keeping this proof
    // focused on the detached-run callback rather than flow-node execution.
    let callback_flow = post_json_rpc(
        &rpc_base,
        71_030,
        "openhuman.flows_create",
        json!({
            "name": "Copilot RunFlow callback probe",
            "graph": {
                "nodes": [{"id":"trigger","kind":"trigger","name":"Manual"}],
                "edges": []
            }
        }),
    )
    .await;
    let callback_flow_id = peel_logs_envelope(assert_no_jsonrpc_error(
        &callback_flow,
        "flows_create for Copilot RunFlow callback",
    ))
    .get("id")
    .and_then(Value::as_str)
    .expect("saved callback flow id")
    .to_string();
    let callback_graph = json!({
        "nodes": [{"id":"trigger","kind":"trigger","name":"Manual"}],
        "edges": []
    });

    fn tool_result_with_marker(requests: &[Value], marker: &str) -> Option<String> {
        requests
            .iter()
            .filter_map(|request| request.pointer("/body/messages").and_then(Value::as_array))
            .flatten()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
            .filter_map(|message| message.get("content").and_then(Value::as_str))
            .find(|content| content.contains(marker))
            .map(str::to_string)
    }

    clear_forced_chat_completions();
    with_chat_completion_requests(|requests| requests.clear());
    push_forced_chat_completion_when(
        "run_flow",
        forced_tool_call_completion("run_flow", json!({ "flow_id": "missing-copilot-flow" })),
    );
    push_forced_chat_completion_when(
        "Could not start flow",
        forced_text_completion("That flow is not saved, so I could not run it."),
    );
    let missing_run = post_json_rpc(
        &rpc_base,
        71_031,
        "openhuman.flows_build",
        json!({
            "mode": "revise",
            "flow_id": "missing-copilot-flow",
            "graph": callback_graph,
            "instruction": "The user confirmed a test run; attempt the saved flow now.",
            "thread_id": "e2e-copilot-runflow-missing",
            "request_id": "req-copilot-runflow-missing"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&missing_run, "Copilot run_flow missing-flow path");
    let missing_requests = with_chat_completion_requests(|requests| requests.clone());
    assert!(
        missing_requests.iter().any(|request| {
            request
                .pointer("/body/tools")
                .and_then(Value::as_array)
                .is_some_and(|tools| {
                    tools.iter().any(|tool| {
                        tool.pointer("/function/name").and_then(Value::as_str) == Some("run_flow")
                    })
                })
        }),
        "streaming Copilot builder must expose the real run_flow tool"
    );
    let missing_tool_result = tool_result_with_marker(&missing_requests, "Could not start flow")
        .expect("module tool result reports the missing saved flow");
    assert!(
        missing_tool_result.contains("missing-copilot-flow"),
        "missing flow result should identify the rejected id: {missing_tool_result}"
    );

    clear_forced_chat_completions();
    with_chat_completion_requests(|requests| requests.clear());
    push_forced_chat_completion_when(
        "run_flow",
        forced_tool_call_completion("run_flow", json!({ "flow_id": callback_flow_id })),
    );
    push_forced_chat_completion_when(
        "workflow_run_started",
        forced_text_completion("The confirmed test run started."),
    );
    let started_run = post_json_rpc(
        &rpc_base,
        71_032,
        "openhuman.flows_build",
        json!({
            "mode": "revise",
            "flow_id": callback_flow_id,
            "graph": callback_graph,
            "instruction": "The user confirmed a test run; run this saved flow now.",
            "thread_id": "e2e-copilot-runflow-saved",
            "request_id": "req-copilot-runflow-saved"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&started_run, "Copilot run_flow saved-flow path");
    let started_requests = with_chat_completion_requests(|requests| requests.clone());
    let started_tool_result = tool_result_with_marker(&started_requests, "workflow_run_started")
        .expect("module tool result reports a detached run");
    let started_payload: Value =
        serde_json::from_str(&started_tool_result).expect("run_flow result is JSON");
    assert_eq!(started_payload["type"], "workflow_run_started");
    assert_eq!(started_payload["flow_id"], callback_flow_id);
    assert_eq!(started_payload["status"], "running");
    assert_eq!(started_payload["detached"], true);
    assert!(
        started_payload["run_id"]
            .as_str()
            .is_some_and(|run_id| !run_id.is_empty()),
        "detached tool result must contain its run id: {started_payload}"
    );

    // Hygiene: never let a leftover scripted completion bleed into another test
    // sharing this process (the FIFO is a process-global static).
    clear_forced_chat_completions();
    mock_join.abort();
    rpc_join.abort();
}

/// Approval park + DENY over JSON-RPC (issue G4): a gate with both a `main`
/// edge (→ `downstream`) and an `error` edge (→ `recover`). Resuming with the
/// gate in `rejections` routes the denied gate's error item to `recover`, and
/// `downstream` must not run.
#[cfg(feature = "flows")]
#[tokio::test]
async fn json_rpc_flows_validate_wire_shape() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let (rpc_base, _tmp, api_join, rpc_join, _guards) = boot_flows_rpc_env().await;

    let valid_graph = json!({
        "name": "wire-valid",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger", "config": { "trigger_kind": "webhook" } }
        ],
        "edges": []
    });
    let validate = post_json_rpc(
        &rpc_base,
        9501,
        "openhuman.flows_validate",
        json!({ "graph": valid_graph }),
    )
    .await;
    let v = peel_logs_envelope(assert_no_jsonrpc_error(&validate, "flows_validate"));
    assert_eq!(v.get("valid").and_then(Value::as_bool), Some(true));
    assert!(v.get("warnings").and_then(Value::as_array).is_some());

    let invalid_graph = json!({
        "name": "no-trigger",
        "nodes": [ { "id": "a", "kind": "output_parser", "name": "A" } ],
        "edges": []
    });
    let validate_bad = post_json_rpc(
        &rpc_base,
        9502,
        "openhuman.flows_validate",
        json!({ "graph": invalid_graph }),
    )
    .await;
    let vb = peel_logs_envelope(assert_no_jsonrpc_error(&validate_bad, "flows_validate"));
    assert_eq!(vb.get("valid").and_then(Value::as_bool), Some(false));
    assert!(vb.get("errors").and_then(Value::as_array).is_some());

    // Preserve serde's member path and detail across the JSON-RPC envelope.
    let malformed_graph = json!({
        "inputs": [ { "type": "string" } ]
    });
    let validate_malformed = post_json_rpc(
        &rpc_base,
        9503,
        "openhuman.flows_validate",
        json!({ "graph": malformed_graph }),
    )
    .await;
    let malformed = peel_logs_envelope(assert_no_jsonrpc_error(
        &validate_malformed,
        "flows_validate malformed graph",
    ));
    assert_eq!(malformed.get("valid").and_then(Value::as_bool), Some(false));
    let errors = malformed
        .get("errors")
        .and_then(Value::as_array)
        .expect("errors");
    assert_eq!(errors.len(), 1, "malformed graph has one parse error");
    let error = errors[0].as_str().expect("string error");
    assert!(error.contains("inputs[0]"), "missing member path: {error}");
    assert!(
        error.contains("missing field"),
        "missing serde detail: {error}"
    );

    api_join.abort();
    rpc_join.abort();
}

/// `openhuman.flows_import` returns a native graph without persisting a flow row.
#[cfg(feature = "flows")]
#[tokio::test]
async fn json_rpc_flows_import_does_not_persist() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let (rpc_base, _tmp, api_join, rpc_join, _guards) = boot_flows_rpc_env().await;

    // 1. Native import — a valid tinyflows graph, warning-free.
    let native = json!({
        "name": "native-flow",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger", "config": { "trigger_kind": "manual" } }
        ],
        "edges": []
    });
    let imp = post_json_rpc(
        &rpc_base,
        9601,
        "openhuman.flows_import",
        json!({ "graph": native, "format": "native" }),
    )
    .await;
    let out = peel_logs_envelope(assert_no_jsonrpc_error(&imp, "flows_import native"));
    assert_eq!(
        out.get("graph")
            .and_then(|g| g.get("name"))
            .and_then(Value::as_str),
        Some("native-flow")
    );
    assert!(
        out.get("warnings")
            .and_then(Value::as_array)
            .expect("warnings")
            .is_empty(),
        "a clean native import has no warnings"
    );

    // Import never persists — no flow row was created.
    let list = post_json_rpc(&rpc_base, 9602, "openhuman.flows_list", json!({})).await;
    let flows = peel_logs_envelope(assert_no_jsonrpc_error(&list, "flows_list"))
        .as_array()
        .expect("flows array")
        .len();
    assert_eq!(flows, 0, "flows_import must not persist a flow");

    api_join.abort();
    rpc_join.abort();
}

/// `openhuman.flows_list_connections` (PHASE 2): the connection picker source.
/// Aggregates Composio connected accounts + stored HTTP credentials into a flat
/// list of `connection_ref` + display + kind — and NEVER any secret material.
///
/// We seed one named HTTP credential (a bearer token) through the same
/// host-side store the RPC reads, then assert the RPC surfaces it as
/// `http_cred:<name>` with `kind = "http"` and that the token value never
/// appears anywhere in the RPC payload. The Composio half is exercised for
/// fault-tolerance: the mock upstream has no connected-accounts route, so the
/// Composio source fails and is tolerated (the RPC still returns the HTTP half
/// rather than erroring).
#[cfg(feature = "flows")]
#[tokio::test]
async fn json_rpc_flows_list_connections_aggregates_secret_free() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let (rpc_base, _tmp, api_join, rpc_join, _guards) = boot_flows_rpc_env().await;

    // Seed an HTTP credential through the same encrypted-at-rest store the op
    // reads (config resolves under the guarded HOME set by boot_flows_rpc_env).
    let seed_config = openhuman_core::config::load_config_with_timeout()
        .await
        .expect("load config to seed http_cred");
    const SECRET: &str = "sk_live_flows_list_connections_seed";
    openhuman_core::flows::http_credentials::HttpCredentialsStore::from_config(&seed_config)
        .upsert(&openhuman_core::flows::http_credentials::HttpCredential::bearer("stripe", SECRET))
        .await
        .expect("seed http_cred");

    let resp = post_json_rpc(
        &rpc_base,
        9330,
        "openhuman.flows_list_connections",
        json!({}),
    )
    .await;
    let raw = assert_no_jsonrpc_error(&resp, "flows_list_connections");

    // The seeded secret must never appear anywhere in the RPC response.
    let raw_str = raw.to_string();
    assert!(
        !raw_str.contains(SECRET),
        "secret leaked into flows_list_connections payload: {raw_str}"
    );

    let connections = peel_logs_envelope(raw)
        .as_array()
        .expect("connections is an array")
        .clone();

    let stripe = connections
        .iter()
        .find(|c| c.get("connection_ref").and_then(Value::as_str) == Some("http_cred:stripe"))
        .expect("seeded http_cred surfaced in picker");
    assert_eq!(stripe.get("kind").and_then(Value::as_str), Some("http"));
    assert_eq!(stripe.get("scheme").and_then(Value::as_str), Some("bearer"));
    assert!(
        stripe.get("display").and_then(Value::as_str).is_some(),
        "http_cred entry must carry a display label"
    );

    api_join.abort();
    rpc_join.abort();
}

// Task 4 / #3090: when a web-chat request is sent with
// `speak_reply: true`, `run_chat_task` should drive the agent's final text
// through `voice::reply_speech::synthesize_reply` after the turn completes.
//
// We activate the [`reply_speech::test_seam`] short-circuit via the
// `OPENHUMAN_TEST_REPLY_SPEECH_SEAM` env var so the call is recorded
// without contacting the ElevenLabs proxy.

// ─────────────────────────────────────────────────────────────────────────────
// #5846 — the two authoring-gate behaviour changes, driven end-to-end through
// the real strict-mode create path.
//
// `flows_create` with `strict: true` runs `ops::strict_gate`
// (`flows/schemas_handlers.rs:18`) -> `run_builder_gates`
// (`flows/ops_part_01.rs:509`) -> `validate_binding_resolvability`
// (`flows/ops_part_02.rs:433`) -> `tinyflows::gates::failures`, the stack
// #5846 evicted upstream.
//
// `ops_tests_part_06_tests.rs` already pins the gate FUNCTION at unit level.
// What no test covered is that the evicted gate is still WIRED into the strict
// RPC path — the eviction moved the implementation out of this repository, so
// the wiring is exactly the thing that can now rot without a local test failing.
//
// A NOTE ON #5846's SUMMARY. The PR body reads "a workflow whose `tool_call`
// arg binds to a schema-less agent is now refused". Taken literally that is not
// what the gate does, deliberately: an agent with NO `output_parser.schema` is
// skipped because "the field may exist, so it is unverifiable rather than
// guaranteed invalid" (`gates/mod.rs:234-239`). Only a field absent from a
// schema that EXISTS is refused. These tests pin the implemented contract, and
// the carve-out gets its own case precisely because the PR wording invites
// someone to "fix" it away.
// ─────────────────────────────────────────────────────────────────────────────

/// A `tool_call` arg reading a field the target agent's declared schema omits.
#[cfg(feature = "flows")]
fn graph_binding_to_undeclared_agent_field() -> Value {
    json!({
        "name": "undeclared-field",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "prompt": "summarize",
                "output_parser": { "schema": { "type": "object",
                  "properties": { "summary": { "type": "string" } } } } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    })
}

#[cfg(feature = "flows")]
#[tokio::test]
async fn json_rpc_flows_strict_create_refuses_binding_to_undeclared_agent_field() {
    let _env_lock = json_rpc_e2e_env_lock_async().await;
    let (rpc_base, _tmp, api_join, rpc_join, _guards) = boot_flows_rpc_env().await;

    let create = post_json_rpc(
        &rpc_base,
        9801,
        "openhuman.flows_create",
        json!({
            "name": "undeclared-field",
            "graph": graph_binding_to_undeclared_agent_field(),
            "strict": true
        }),
    )
    .await;

    let refusal = create
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("strict create must be refused, got: {create}"));
    assert!(
        refusal.contains("output_parser.schema"),
        "the refusal must come from the undeclared-field gate, got: {refusal}"
    );
    assert!(
        refusal.contains("channel"),
        "the refusal must name the offending field, got: {refusal}"
    );

    rpc_join.abort();
    api_join.abort();
}

// ---------------------------------------------------------------------------
// Migration import guard: refusing the null memory driver (#5799)
// ---------------------------------------------------------------------------
