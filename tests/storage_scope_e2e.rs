//! Background work reaches the records an agent keeps in its own storage
//! scope.
//!
//! A job scheduled from inside an agent's context is stored in that agent's
//! scope, so the `local` pass a background loop makes on its own never sees
//! it; `storage::agents::for_each_scope` visits the agent too — through its
//! live context while it exists, and through the agent id the backend
//! recorded once it is gone (a restarted process).
//!
//! Its own test binary because it installs a backend into the process-wide
//! storage slot and boots a core. The tests take turns on the slot
//! (`support/storage_case.rs`).

use std::sync::Arc;

use openhuman_core::config::Config;
use openhuman_core::core::runtime::{
    AgentContextRegistry, ContextOverlay, CoreBuilder, CoreContext, DomainSet, ServiceSet,
};
use openhuman_core::cron::{self, Schedule};
use openhuman_core::storage::agents::{find_owner, for_each_scope, within_agent};
use openhuman_core::HostKind;

#[path = "support/tinyhumans_boot.rs"]
mod tinyhumans_boot;

#[path = "support/storage_case.rs"]
mod storage_case;

use storage_case::Case;

fn job_names(config: &Config) -> Vec<String> {
    cron::list_jobs(config)
        .unwrap()
        .into_iter()
        .filter_map(|job| job.name)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_work_visits_every_agent_scope() {
    let case = Case::open();
    let workspace = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: workspace.path().join("workspace"),
        config_path: workspace.path().join("config.toml"),
        ..Config::default()
    };
    tinyhumans_boot::boot();
    let _runtime = CoreBuilder::new(HostKind::Library)
        .config(config.clone())
        .services(ServiceSet::none())
        .domains(DomainSet::none())
        .build()
        .await
        .unwrap();
    case.install();

    let agent = CoreContext::current().unwrap().derive_with(
        ContextOverlay::new(config.clone(), DomainSet::none(), Default::default())
            .session_agent("agent-e2e"),
    );
    // How a host's agent becomes known to background work (embed does this
    // when it builds an agent).
    AgentContextRegistry::register("agent-e2e", &agent);
    // The store calls block on storage's own bridge thread, so they are
    // made straight from the agent's task, where its context is in scope.
    CoreContext::scope(Arc::clone(&agent), async {
        cron::add_shell_job(
            &config,
            Some("agent-job".to_string()),
            Schedule::Every { every_ms: 60_000 },
            "echo hi",
        )
        .unwrap();
    })
    .await;

    // The agent's job is invisible to the `local` scope.
    assert!(job_names(&config).is_empty());

    // Visited through the live agent context …
    let live = for_each_scope(
        &openhuman_core::core::runtime::DispatchCtx::host_default(),
        "e2e",
        |_| async { job_names(&config) },
    )
    .await;
    assert!(
        live.contains(&(Some("agent-e2e".to_string()), vec!["agent-job".to_string()])),
        "{live:?}"
    );

    // An event naming only the job resolves to its owner, and handling it
    // there sees the job.
    let job_id = within_agent(
        &openhuman_core::core::runtime::DispatchCtx::host_default(),
        Some("agent-e2e"),
        async { cron::list_jobs(&config).unwrap()[0].id.clone() },
    )
    .await
    .expect("the live agent has a context");
    let owner = find_owner(
        &openhuman_core::core::runtime::DispatchCtx::host_default(),
        "e2e",
        |_| async { Ok(cron::get_job(&config, &job_id).is_ok()) },
    )
    .await;
    assert_eq!(owner, Ok(Some(Some("agent-e2e".to_string()))));
    let missing = find_owner(
        &openhuman_core::core::runtime::DispatchCtx::host_default(),
        "e2e",
        |_| async { Ok(cron::get_job(&config, "nope").is_ok()) },
    )
    .await;
    assert_eq!(missing, Ok(None));

    // … and, once the agent is gone, through the id the backend recorded.
    assert!(AgentContextRegistry::deregister("agent-e2e", &agent));
    drop(agent);
    let recorded = for_each_scope(
        &openhuman_core::core::runtime::DispatchCtx::host_default(),
        "e2e",
        |_| async { job_names(&config) },
    )
    .await;
    assert!(
        recorded.contains(&(Some("agent-e2e".to_string()), vec!["agent-job".to_string()])),
        "{recorded:?}"
    );
    assert!(recorded.contains(&(None, Vec::new())), "{recorded:?}");
}
