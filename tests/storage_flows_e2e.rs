//! Cron jobs, the flow catalog and flow state on a configured storage
//! backend, end to end through the core's public functions.
//!
//! Its own test binary because it installs a backend into the process-wide
//! storage slot, which would reroute every other suite's stores in a shared
//! process. Each driver runs as its own test case (see `support/storage_drivers.rs`), and the cases take turns on the slot.

use openhuman_core::config::Config;
use openhuman_core::cron::{self, Schedule};
use openhuman_core::flows;
use openhuman_core::flows::tinyflows::state::FlowState;
use serde_json::json;

#[macro_use]
#[path = "support/storage_drivers.rs"]
mod storage_drivers;

use storage_drivers::Case;

async fn a_configured_backend_holds_cron_flows_and_flow_state(case: Case) {
    let workspace = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: workspace.path().to_path_buf(),
        ..Config::default()
    };
    case.install();

    // Cron: the store functions run on blocking threads, as the scheduler does.
    let cron_config = config.clone();
    tokio::task::spawn_blocking(move || {
        let job = cron::add_shell_job(
            &cron_config,
            Some("tick".to_string()),
            Schedule::Every { every_ms: 60_000 },
            "echo hi",
        )
        .unwrap();
        assert_eq!(cron::list_jobs(&cron_config).unwrap().len(), 1);
        let now = chrono::Utc::now();
        cron::record_run(&cron_config, &job.id, now, now, "ok", Some("hi"), 3).unwrap();
        assert_eq!(cron::list_runs(&cron_config, &job.id, 10).unwrap().len(), 1);
        cron::remove_job(&cron_config, &job.id).unwrap();
        assert!(cron::list_jobs(&cron_config).unwrap().is_empty());
    })
    .await
    .unwrap();

    // The flow catalog, through the RPC operations.
    let graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Summarize", "config": { "prompt": "hi" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    });
    let flow = flows::ops::flows_create(&config, "Digest".to_string(), graph, false)
        .await
        .unwrap()
        .value;
    let listed = flows::ops::flows_list(&config).await.unwrap().value;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, flow.id);

    // Flow state resolves to the backend, and its records read back through
    // the catalog's KV (the engine/dedup sharing is unit-tested in core).
    let state_config = config.clone();
    tokio::task::spawn_blocking(move || {
        assert!(matches!(
            FlowState::open(&state_config, "flow:digest"),
            FlowState::Documents(_)
        ));
        flows::kv_set(&state_config, "flow:digest", "seen", &json!(["a"])).unwrap();
        assert_eq!(
            flows::kv_get(&state_config, "flow:digest", "seen").unwrap(),
            Some(json!(["a"]))
        );
    })
    .await
    .unwrap();

    // The delegation graph's checkpointer lives on the backend too.
    let delegation_config = config.clone();
    openhuman_core::agent::orchestration::open_delegation_checkpointer(&delegation_config).unwrap();

    // A job's scheduling policy lives on the backend too.
    let policy_config = config.clone();
    tokio::task::spawn_blocking(move || {
        use openhuman_core::cron::{CronJobPatch, JobPolicy};
        let set_policy = |config: &_, id: &str, policy: JobPolicy| {
            cron::update_job(
                config,
                id,
                CronJobPatch {
                    policy: Some(policy),
                    ..CronJobPatch::default()
                },
            )
            .unwrap()
            .policy
        };
        let job = cron::add_shell_job(
            &policy_config,
            Some("policy".to_string()),
            Schedule::Every { every_ms: 60_000 },
            "echo hi",
        )
        .unwrap();
        let wanted = JobPolicy {
            retries: Some(0),
            single_flight: true,
        };
        assert_eq!(set_policy(&policy_config, &job.id, wanted), wanted);
        assert_eq!(
            cron::get_job(&policy_config, &job.id).unwrap().policy,
            wanted
        );
        assert_eq!(
            set_policy(&policy_config, &job.id, JobPolicy::default()),
            JobPolicy::default()
        );
    })
    .await
    .unwrap();

    for path in [
        "cron/jobs.db",
        "cron/schedule.db",
        "flows/flows.db",
        "flows/catalog.db",
        "graph_checkpoints.db",
    ] {
        assert!(
            !workspace.path().join(path).exists(),
            "{path} was not written"
        );
    }

    // Without a backend the workspace's own databases are used.
    assert!(openhuman_core::storage::clear());
    let fallback = config.clone();
    tokio::task::spawn_blocking(move || {
        assert!(matches!(
            FlowState::open(&fallback, "flow:digest"),
            FlowState::Documents(_)
        ));
    })
    .await
    .unwrap();
    assert!(workspace.path().join("storage/storage.db").exists());
    // The flow written to the backend is not in the workspace's catalog.
    assert!(flows::ops::flows_list(&config)
        .await
        .unwrap()
        .value
        .is_empty());
    openhuman_core::agent::orchestration::open_delegation_checkpointer(&config).unwrap();
}

/// The stores tinyflows used to write (`jobs.db`, `flows.db`, `checkpoints.db`,
/// `drafts/`) are copied into a configured backend by the boot-time import,
/// read-only: the legacy files are byte-identical afterwards.
async fn legacy_tinyflows_files_are_imported_into_a_configured_backend(case: Case) {
    let workspace = tempfile::tempdir().unwrap();
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../openhuman-core/src/storage/import/fixtures/tinyflows/workspace");
    copy_tree(&fixtures, workspace.path());
    let config = Config {
        workspace_dir: workspace.path().to_path_buf(),
        ..Config::default()
    };
    let digest = |root: &std::path::Path| {
        ["cron/jobs.db", "flows/flows.db", "flows/checkpoints.db"]
            .map(|file| std::fs::read(root.join(file)).unwrap())
    };
    let before = digest(workspace.path());
    case.install();

    for report in openhuman_core::storage::import::run(
        &openhuman_core::core::runtime::DispatchCtx::process_default(),
        &config,
    )
    .await
    {
        report.outcome.unwrap();
    }

    let listed = flows::ops::flows_list(&config).await.unwrap().value;
    let mut ids: Vec<_> = listed.iter().map(|flow| flow.id.clone()).collect();
    ids.sort();
    assert_eq!(ids, ["flow-alpha", "flow-beta"]);
    let cron_config = config.clone();
    tokio::task::spawn_blocking(move || {
        assert_eq!(cron::list_jobs(&cron_config).unwrap().len(), 3);
    })
    .await
    .unwrap();

    // Run again: the markers say it is done, and nothing is rewritten.
    for report in openhuman_core::storage::import::run(
        &openhuman_core::core::runtime::DispatchCtx::process_default(),
        &config,
    )
    .await
    {
        report.outcome.unwrap();
    }
    assert_eq!(
        digest(workspace.path()),
        before,
        "a legacy file was modified"
    );
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

driver_cases!(
    async a_configured_backend_holds_cron_flows_and_flow_state,
    legacy_tinyflows_files_are_imported_into_a_configured_backend
);
