//! A cron agent job whose `agent_id` names a host-registered agent
//! (`agent::host_agents`) runs as that agent: its definition, its host tools,
//! its context. Any other id still resolves through the registries.

use super::*;
use crate::agent::host_agents::{self, HostAgent, HostAgentResolver};
use crate::core::runtime::{CoreContext, DomainSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

struct Marker;

#[async_trait::async_trait]
impl tinytools::Tool for Marker {
    fn name(&self) -> &str {
        "cron_host_marker"
    }
    fn description(&self) -> &str {
        "a host tool only the host agent has"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
        Ok(tinytools::ToolResult::success("ok"))
    }
}

struct Host {
    config: Config,
    belt_builds: Arc<AtomicUsize>,
}

impl HostAgentResolver for Host {
    fn resolve(&self, agent_id: &str) -> Option<HostAgent> {
        if agent_id != "cron-host-agent" {
            return None;
        }
        let mut definition =
            crate::agent::harness::host_definition::AgentDefinitionRegistry::builtins_only()
                .get("orchestrator")
                .cloned()
                .unwrap();
        definition.id = agent_id.to_string();
        let builds = Arc::clone(&self.belt_builds);
        Some(HostAgent {
            definition,
            config: self.config.clone(),
            host_tools: Some(Arc::new(move |turn| {
                assert_eq!(turn.agent_id(), "cron-host-agent");
                builds.fetch_add(1, Ordering::SeqCst);
                crate::agent::HostTurnTools::advertised(vec![Box::new(Marker)])
            })),
            hooks: Default::default(),
            context: CoreContext::for_test(
                DomainSet::full(),
                Some(self.config.workspace_dir.clone()),
            ),
        })
    }
}

/// `test_config` without the await, so the resolver lock is never held
/// across one.
fn config(tmp: &TempDir) -> Config {
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    Config {
        workspace_dir: workspace.clone(),
        action_dir: workspace,
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

fn agent_job(agent_id: &str) -> CronJob {
    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.agent_id = Some(agent_id.to_string());
    job.prompt = Some("check in".into());
    job
}

#[test]
fn a_cron_job_for_a_host_agent_builds_with_its_host_tools_and_context() {
    let _slot = crate::agent::host_agents::tests::lock();
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let belt_builds = Arc::new(AtomicUsize::new(0));
    let installed: Arc<dyn HostAgentResolver> = Arc::new(Host {
        config: config.clone(),
        belt_builds: Arc::clone(&belt_builds),
    });
    host_agents::install(Arc::clone(&installed));

    let built = build_agent_for_cron_job(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &agent_job("cron-host-agent"),
    );
    host_agents::clear_if(&installed);
    let built = built.expect("the host agent builds");

    assert!(
        built.context.is_some(),
        "the turn runs in the host agent's context"
    );
    assert!(
        belt_builds.load(Ordering::SeqCst) > 0,
        "the host belt was built"
    );
    assert!(built
        .agent
        .visible_tool_specs_arc()
        .iter()
        .any(|spec| spec.name == "cron_host_marker"));
}

#[test]
fn an_id_the_host_does_not_know_falls_back_to_the_registries() {
    let _slot = crate::agent::host_agents::tests::lock();
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let installed: Arc<dyn HostAgentResolver> = Arc::new(Host {
        config: config.clone(),
        belt_builds: Arc::new(AtomicUsize::new(0)),
    });
    host_agents::install(Arc::clone(&installed));
    let built = build_agent_for_cron_job(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &agent_job("orchestrator"),
    );
    host_agents::clear_if(&installed);
    let built = built.expect("the orchestrator builds from the registries");
    assert!(built.context.is_none());
    assert!(!built
        .agent
        .visible_tool_specs_arc()
        .iter()
        .any(|spec| spec.name == "cron_host_marker"));
}
