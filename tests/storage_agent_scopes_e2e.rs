//! Two agents on one storage backend keep their cron jobs and flows apart,
//! the scheduler's agent pass runs each agent's due jobs in that agent's own
//! scope, and a call with no acting agent in single-user mode falls back to
//! the `local` scope.
//!
//! Its own test binary because it installs a backend into the process-wide
//! storage slot and boots a core.

use std::sync::Arc;

use openhuman_core::config::Config;
use openhuman_core::core::runtime::{
    AgentContextRegistry, ContextOverlay, CoreBuilder, CoreContext, DomainSet, ServiceSet,
};
use openhuman_core::cron::{self, Schedule};
use openhuman_core::flows;
use openhuman_core::storage::agents::for_each_scope;
use openhuman_core::storage::{self, Scope};
use openhuman_core::HostKind;
use serde_json::json;

#[path = "support/tinyhumans_boot.rs"]
mod tinyhumans_boot;

#[path = "support/storage_case.rs"]
mod storage_case;

use storage_case::Case;

#[path = "support/cron_policy_failure.rs"]
mod cron_policy_failure;

fn agent_context(config: &Config, agent: &str) -> Arc<CoreContext> {
    let context = CoreContext::current().unwrap().derive_with(
        ContextOverlay::new(config.clone(), DomainSet::none(), Default::default())
            .session_agent(agent),
    );
    AgentContextRegistry::register(agent, &context);
    context
}

fn flow_graph(prompt: &str) -> serde_json::Value {
    json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Work", "config": { "prompt": prompt } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    })
}

/// A job that is due on the next scheduler pass.
fn add_due_job(config: &Config, name: &str) -> cron::CronJob {
    cron::add_shell_job(
        config,
        Some(name.to_string()),
        Schedule::Every { every_ms: 1 },
        "echo scoped",
    )
    .unwrap()
}

fn job_names(config: &Config) -> Vec<String> {
    let mut names: Vec<String> = cron::list_jobs(config)
        .unwrap()
        .into_iter()
        .filter_map(|job| job.name)
        .collect();
    names.sort();
    names
}

async fn flow_names(config: &Config) -> Vec<String> {
    flows::ops::flows_list(config)
        .await
        .unwrap()
        .value
        .into_iter()
        .map(|flow| flow.name)
        .collect()
}

/// Waits (bounded) until the clock has reached the job's next run in the
/// agent's scope, so the scheduler pass finds it due without a fixed sleep.
async fn wait_until_due(ctx: &Arc<CoreContext>, config: &Config, id: &str) {
    for _ in 0..200 {
        let next_run = CoreContext::scope(Arc::clone(ctx), async {
            cron::get_job(config, id).unwrap().next_run
        })
        .await;
        if next_run <= chrono::Utc::now() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("job {id} never became due");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_agents_keep_cron_and_flows_apart() {
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

    let alpha = agent_context(&config, "agent-alpha");
    let beta = agent_context(&config, "agent-beta");

    // Created as agent A.
    let (alpha_job, alpha_flow) = CoreContext::scope(Arc::clone(&alpha), async {
        let job = add_due_job(&config, "alpha-job");
        let flow = flows::ops::flows_create(
            &config,
            "alpha-flow".to_string(),
            flow_graph("alpha"),
            false,
        )
        .await
        .unwrap()
        .value;
        (job, flow)
    })
    .await;

    // Invisible to agent B …
    CoreContext::scope(Arc::clone(&beta), async {
        assert!(job_names(&config).is_empty(), "B sees A's job");
        assert!(cron::get_job(&config, &alpha_job.id).is_err());
        assert!(flow_names(&config).await.is_empty(), "B sees A's flow");
        assert!(
            flows::ops::flows_get(&config, &alpha_flow.id)
                .await
                .is_err(),
            "B can read A's flow by id"
        );
    })
    .await;
    // … and to the operator's `local` scope.
    assert!(job_names(&config).is_empty());
    assert!(flow_names(&config).await.is_empty());

    // Each agent keeps its own under the same names.
    let beta_job = CoreContext::scope(Arc::clone(&beta), async {
        let job = add_due_job(&config, "beta-job");
        flows::ops::flows_create(&config, "beta-flow".to_string(), flow_graph("beta"), false)
            .await
            .unwrap();
        assert_eq!(job_names(&config), vec!["beta-job".to_string()]);
        assert_eq!(flow_names(&config).await, vec!["beta-flow".to_string()]);
        job
    })
    .await;
    CoreContext::scope(Arc::clone(&alpha), async {
        assert_eq!(job_names(&config), vec!["alpha-job".to_string()]);
        assert_eq!(flow_names(&config).await, vec!["alpha-flow".to_string()]);
    })
    .await;

    // The scheduler's agent pass runs each agent's due job under that agent.
    for (ctx, id) in [(&alpha, &alpha_job.id), (&beta, &beta_job.id)] {
        wait_until_due(ctx, &config, id).await;
    }
    cron::scheduler::run_live_agent_pass(2).await;
    for (agent, ctx, own, other) in [
        ("alpha", &alpha, &alpha_job.id, &beta_job.id),
        ("beta", &beta, &beta_job.id, &alpha_job.id),
    ] {
        CoreContext::scope(Arc::clone(ctx), async {
            let job = cron::get_job(&config, own).unwrap();
            assert!(job.last_run.is_some(), "{agent}'s own job did not run");
            assert!(
                cron::get_job(&config, other).is_err(),
                "{agent} can see the other agent's job"
            );
            let runs = cron::list_runs(&config, own, 10).unwrap();
            assert_eq!(runs.len(), 1, "{agent}: one run recorded in its scope");
            assert!(cron::list_runs(&config, other, 10).unwrap().is_empty());
        })
        .await;
    }
    // Nothing ran in the operator scope: it has no jobs.
    assert!(job_names(&config).is_empty());

    // Background visits see the agents' jobs under their own ids.
    let visited = for_each_scope(
        &openhuman_core::core::runtime::DispatchCtx::host_default(),
        "e2e",
        |_| async { job_names(&config) },
    )
    .await;
    assert!(
        visited.contains(&(
            Some("agent-alpha".to_string()),
            vec!["alpha-job".to_string()]
        )),
        "{visited:?}"
    );
    assert!(
        visited.contains(&(Some("agent-beta".to_string()), vec!["beta-job".to_string()])),
        "{visited:?}"
    );

    assert!(AgentContextRegistry::deregister("agent-alpha", &alpha));
    assert!(AgentContextRegistry::deregister("agent-beta", &beta));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_acting_agent_falls_back_to_the_local_scope() {
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

    // Single-user mode, no acting agent: the operator's scope is `local`,
    // and the storage the stores read is that scope's.
    assert_eq!(storage::current_scope().unwrap(), Scope::local());
    assert!(storage::current_scoped().unwrap().is_some());
    assert_eq!(
        storage::scope_from(None, None, false).unwrap(),
        Scope::local()
    );
    // SaaS mode refuses the same call instead of sharing a bucket.
    assert!(storage::scope_from(None, None, true).is_err());

    add_due_job(&config, "operator-job");
    flows::ops::flows_create(
        &config,
        "operator-flow".to_string(),
        flow_graph("op"),
        false,
    )
    .await
    .unwrap();
    assert_eq!(job_names(&config), vec!["operator-job".to_string()]);
    assert_eq!(flow_names(&config).await, vec!["operator-flow".to_string()]);

    // An agent has a scope of its own, distinct from `local`, and does not
    // see what the operator created.
    let agent = agent_context(&config, "agent-gamma");
    CoreContext::scope(Arc::clone(&agent), async {
        let scope = storage::current_scope().unwrap();
        assert_eq!(scope, storage::scope_for_agent("agent-gamma"));
        assert_ne!(scope, Scope::local());
        assert!(job_names(&config).is_empty());
        assert!(flow_names(&config).await.is_empty());
    })
    .await;

    // The operator's records are what the `local` visit of a background loop
    // sees.
    let visited = for_each_scope(
        &openhuman_core::core::runtime::DispatchCtx::host_default(),
        "e2e",
        |_| async { job_names(&config) },
    )
    .await;
    assert!(
        visited.contains(&(None, vec!["operator-job".to_string()])),
        "{visited:?}"
    );
    assert!(AgentContextRegistry::deregister("agent-gamma", &agent));
}
