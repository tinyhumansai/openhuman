use super::*;
use crate::agent::harness::host_definition::AgentDefinition;
use crate::core::runtime::DomainSet;

/// The resolver slot is process-wide; every test in the crate that installs
/// one takes its turn through this lock.
static SLOT: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn lock() -> std::sync::MutexGuard<'static, ()> {
    SLOT.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn definition(id: &str) -> AgentDefinition {
    let mut definition =
        crate::agent::harness::host_definition::AgentDefinitionRegistry::builtins_only()
            .get("orchestrator")
            .cloned()
            .expect("the orchestrator is built in");
    definition.id = id.to_string();
    definition
}

/// Answers for exactly one id, so a resolver left installed by a failing test
/// cannot change the outcome of any other test in the binary.
struct OneAgent {
    id: String,
    config: Config,
}

impl HostAgentResolver for OneAgent {
    fn resolve(&self, agent_id: &str) -> Option<HostAgent> {
        (agent_id == self.id).then(|| HostAgent {
            definition: definition(&self.id),
            config: self.config.clone(),
            host_tools: None,
            hooks: Default::default(),
            context: CoreContext::for_test(DomainSet::full(), None),
        })
    }
}

fn resolver(id: &str) -> Arc<dyn HostAgentResolver> {
    Arc::new(OneAgent {
        id: id.to_string(),
        config: Config::default(),
    })
}

#[test]
fn nothing_resolves_without_an_installed_resolver() {
    let _slot = lock();
    let installed = resolver("host-agents-none");
    install(Arc::clone(&installed));
    assert!(clear_if(&installed));
    assert!(resolve("host-agents-none").is_none());
}

#[test]
fn an_installed_resolver_answers_for_its_own_agents_only() {
    let _slot = lock();
    let installed = resolver("host-agents-known");
    install(Arc::clone(&installed));
    let hit = resolve("host-agents-known").expect("the installed resolver knows this id");
    assert_eq!(hit.definition.id, "host-agents-known");
    assert!(resolve("host-agents-unknown").is_none());
    assert!(clear_if(&installed));
}

#[test]
fn clear_if_leaves_a_replacement_installed_by_someone_else() {
    let _slot = lock();
    let first = resolver("host-agents-first");
    let second = resolver("host-agents-second");
    install(Arc::clone(&first));
    let previous = install(Arc::clone(&second));
    assert!(previous.is_some_and(|previous| Arc::ptr_eq(&previous, &first)));
    assert!(
        !clear_if(&first),
        "the first owner no longer holds the slot"
    );
    assert!(resolve("host-agents-second").is_some());
    assert!(clear_if(&second));
    assert!(resolve("host-agents-second").is_none());
}

#[test]
fn a_host_agent_debug_names_the_agent_and_hides_the_rest() {
    let host = HostAgent {
        definition: definition("host-agents-debug"),
        config: Config::default(),
        host_tools: None,
        hooks: Default::default(),
        context: CoreContext::for_test(DomainSet::full(), None),
    };
    let rendered = format!("{host:?}");
    assert!(rendered.contains("host-agents-debug"), "{rendered}");
    assert!(rendered.contains("host_tools: false"), "{rendered}");
}

#[tokio::test]
async fn scope_runs_the_future_under_the_agents_context() {
    let workspace = tempfile::tempdir().unwrap();
    let context = CoreContext::for_test(DomainSet::none(), Some(workspace.path().to_path_buf()));
    let host = HostAgent {
        definition: definition("host-agents-scope"),
        config: Config::default(),
        host_tools: None,
        hooks: Default::default(),
        context: Arc::clone(&context),
    };
    let seen = host
        .scope(async { CoreContext::current().map(|ctx| ctx.workspace_dir()) })
        .await;
    assert_eq!(
        seen.expect("a context is in scope").unwrap(),
        workspace.path()
    );
}
