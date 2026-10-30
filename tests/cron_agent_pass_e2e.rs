//! `run_agent_pass` runs one live agent's due jobs to completion and leaves
//! every other agent's due jobs for their own pass, and answers `None` for an
//! agent that is not live.
//!
//! Its own test binary because it installs a backend into the process-wide
//! storage slot and boots a core.

use std::sync::Arc;

use openhuman_core::config::Config;
use openhuman_core::core::runtime::{
    AgentContextRegistry, ContextOverlay, CoreBuilder, CoreContext, DomainSet, ServiceSet,
};
use openhuman_core::cron::{self, Schedule};
use openhuman_core::HostKind;

#[path = "support/tinyhumans_boot.rs"]
mod tinyhumans_boot;

#[path = "support/storage_case.rs"]
mod storage_case;

use storage_case::Case;

fn agent_context(config: &Config, agent: &str) -> Arc<CoreContext> {
    let context = CoreContext::current().unwrap().derive_with(
        ContextOverlay::new(config.clone(), DomainSet::none(), Default::default())
            .session_agent(agent),
    );
    AgentContextRegistry::register(agent, &context);
    context
}

/// Adds, in `ctx`'s scope, a job that is due on the next pass, and waits
/// (bounded) until the clock has reached its next run.
async fn add_due_job(ctx: &Arc<CoreContext>, config: &Config, name: &str) -> String {
    let id = CoreContext::scope(Arc::clone(ctx), async {
        cron::add_shell_job(
            config,
            Some(name.to_string()),
            Schedule::Every { every_ms: 1 },
            "echo scoped",
        )
        .unwrap()
        .id
    })
    .await;
    for _ in 0..200 {
        let next_run = CoreContext::scope(Arc::clone(ctx), async {
            cron::get_job(config, &id).unwrap().next_run
        })
        .await;
        if next_run <= chrono::Utc::now() {
            return id;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("job {id} never became due");
}

async fn runs_of(ctx: &Arc<CoreContext>, config: &Config, id: &str) -> usize {
    CoreContext::scope(Arc::clone(ctx), async {
        cron::list_runs(config, id, 10).unwrap().len()
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_pass_runs_only_that_agents_due_jobs() {
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

    let alpha = agent_context(&config, "pass-alpha");
    let beta = agent_context(&config, "pass-beta");
    let alpha_job = add_due_job(&alpha, &config, "alpha-job").await;
    let beta_job = add_due_job(&beta, &config, "beta-job").await;

    let dispatched = cron::scheduler::run_agent_pass("pass-alpha").await;

    assert_eq!(dispatched, Some(1), "alpha's one due job is dispatched");
    assert_eq!(runs_of(&alpha, &config, &alpha_job).await, 1);
    let ran = CoreContext::scope(Arc::clone(&alpha), async {
        cron::get_job(&config, &alpha_job).unwrap().last_run
    })
    .await;
    assert!(ran.is_some(), "alpha's job records its run");
    assert_eq!(
        runs_of(&beta, &config, &beta_job).await,
        0,
        "beta's due job waits for beta's own pass"
    );

    assert_eq!(cron::scheduler::run_agent_pass("pass-beta").await, Some(1));
    assert_eq!(runs_of(&beta, &config, &beta_job).await, 1);

    assert_eq!(
        cron::scheduler::run_agent_pass("pass-nobody").await,
        None,
        "an agent that is not live has no pass"
    );
}
