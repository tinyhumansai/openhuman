//! A cron agent job whose agent names a `scheduled_workspace` runs rooted
//! there: the run carries it as its workspace descriptor, so a sandboxed shell
//! asks for that one root, never the action dir that holds the core workspace
//! (#8674). A root over the core workspace fails the run before it starts.

use super::*;
use crate::agent::host_agents::{self, HostAgent, HostAgentResolver};
use crate::core::runtime::{CoreContext, DomainSet};
use tempfile::TempDir;

const AGENT_ID: &str = "cron-rooted-agent";

struct Rooted {
    config: Config,
    root: &'static str,
}

impl HostAgentResolver for Rooted {
    fn resolve(&self, agent_id: &str) -> Option<HostAgent> {
        if agent_id != AGENT_ID {
            return None;
        }
        let mut definition =
            crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only()
                .get("orchestrator")
                .cloned()
                .unwrap();
        definition.id = agent_id.to_string();
        definition.sandbox_mode = tinyagents_definition::SandboxMode::Sandboxed;
        definition.extensions.scheduled_workspace = Some(self.root.to_string());
        let context =
            CoreContext::for_test(DomainSet::full(), Some(self.config.workspace_dir.clone()));
        crate::agent::session_host::test_support::install_model_fixture_in(context.extensions());
        Some(HostAgent {
            definition,
            config: self.config.clone(),
            host_tools: None,
            hooks: Default::default(),
            context,
        })
    }
}

/// `<root>/brain` is the core workspace and `<root>` the action dir: the
/// layout whose every sandboxed shell spawn TinySecurity refuses.
async fn checkout(tmp: &TempDir) -> Config {
    let root = tmp.path().canonicalize().unwrap();
    let workspace = root.join("brain");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(root.join("work/job")).unwrap();
    std::fs::create_dir_all(root.join("openhuman")).unwrap();
    crate::storage::local::use_persistent_default_for_test(&workspace);
    let mut config = Config {
        workspace_dir: workspace,
        action_dir: root.clone(),
        config_path: root.join("openhuman/config.toml"),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    config.modules.source = crate::config::schema::ModuleSourceMode::Source;
    #[cfg(feature = "security-module")]
    crate::modules::security::ensure_loaded(&config)
        .await
        .expect("the staged TinySecurity module should load for scheduler tests");
    crate::credentials::api_key::store_api_key(&config, "cron-rooted-test-key")
        .await
        .expect("store the managed inference fixture credential");
    config
}

async fn build(
    config: &Config,
    root: &'static str,
) -> anyhow::Result<crate::cron::scheduler::agent_run::BuiltCronAgent> {
    let installed: Arc<dyn HostAgentResolver> = Arc::new(Rooted {
        config: config.clone(),
        root,
    });
    host_agents::install(Arc::clone(&installed));
    let mut job = test_job("");
    job.job_type = JobType::Agent;
    job.agent_id = Some(AGENT_ID.to_string());
    job.prompt = Some("build the job".into());
    let built =
        build_agent_for_cron_job(&crate::core::runtime::DispatchCtx::ambient(), config, &job).await;
    host_agents::clear_if(&installed);
    built
}

#[tokio::test]
async fn a_rooted_cron_agent_plans_its_shell_with_only_that_root_writable() {
    let _slot = crate::agent::host_agents::tests::lock();
    let tmp = TempDir::new().unwrap();
    let config = checkout(&tmp).await;
    let job_root = config.action_dir.join("work/job");

    let built = build(&config, "work/job")
        .await
        .expect("the rooted agent builds");
    let workspace = built
        .agent
        .live_workspace_descriptor()
        .expect("the run carries its root as the workspace descriptor");
    assert_eq!(workspace.root, job_root);

    assert_shell_asks_for_only(&config, &workspace.root);
}

/// The shell roots its sandbox policy at the run's workspace descriptor
/// (`ShellTool::effective_action_dir_for_context`), and the run's own config
/// has the same root as its action dir; that policy is what a spawn asks
/// TinySecurity to grant.
#[cfg(feature = "security-module")]
fn assert_shell_asks_for_only(config: &Config, job_root: &std::path::Path) {
    let policy = crate::sandbox::policy::route_for_mode(
        tinyagents_definition::SandboxMode::Sandboxed,
        job_root,
        &config.workspace_dir,
        &config.sandbox,
        &config.runtime,
        false,
    );
    let writable: Vec<_> = crate::sandbox::spawn_plan::grants(&policy)
        .expect("grants")
        .into_iter()
        .filter(|grant| grant.write)
        .map(|grant| grant.path)
        .collect();
    assert_eq!(writable, [job_root.to_string_lossy().to_string()]);
}

#[cfg(not(feature = "security-module"))]
fn assert_shell_asks_for_only(_config: &Config, _job_root: &std::path::Path) {}

#[tokio::test]
async fn a_cron_agent_rooted_over_the_core_workspace_fails_before_it_runs() {
    let _slot = crate::agent::host_agents::tests::lock();
    let tmp = TempDir::new().unwrap();
    let config = checkout(&tmp).await;

    for root in [".", "brain"] {
        let error = match build(&config, root).await {
            Ok(_) => panic!("{root}: a root over the core workspace must not build"),
            Err(error) => format!("{error:#}"),
        };
        assert!(error.contains("core workspace"), "{root}: {error}");
    }
}
