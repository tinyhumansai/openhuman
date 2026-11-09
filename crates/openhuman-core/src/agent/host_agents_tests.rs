use super::*;
use crate::agent_registry::host_definition::AgentDefinition;
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
        crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only()
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

/// Opens whatever it is asked for under `opens_as`, and reports a live
/// `live` agent with a lease that flips `released` when dropped.
struct Leasing {
    live: String,
    opens_as: String,
    released: Arc<std::sync::atomic::AtomicBool>,
}

struct Lease(Arc<std::sync::atomic::AtomicBool>);

impl Drop for Lease {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Leasing {
    fn agent(id: &str) -> HostAgent {
        HostAgent {
            definition: definition(id),
            config: Config::default(),
            host_tools: None,
            hooks: Default::default(),
            context: CoreContext::for_test(DomainSet::full(), None),
        }
    }
}

impl HostAgentResolver for Leasing {
    fn resolve(&self, agent_id: &str) -> Option<HostAgent> {
        (agent_id == self.live).then(|| Self::agent(agent_id))
    }

    fn resolve_leased(&self, agent_id: &str) -> Option<OpenedHostAgent> {
        self.resolve(agent_id)
            .map(|agent| OpenedHostAgent::new(agent).with_lease(Lease(Arc::clone(&self.released))))
    }

    fn open<'a>(
        &'a self,
        _agent_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<OpenedHostAgent>> + Send + 'a>> {
        Box::pin(async move { Some(OpenedHostAgent::new(Self::agent(&self.opens_as))) })
    }
}

#[tokio::test]
async fn resolve_or_open_refuses_an_agent_opened_under_another_id() {
    let _slot = lock();
    let installed: Arc<dyn HostAgentResolver> = Arc::new(Leasing {
        live: "host-agents-live".into(),
        opens_as: "host-agents-other".into(),
        released: Arc::default(),
    });
    install(Arc::clone(&installed));
    let asked_for_other = resolve_or_open("host-agents-wanted").await;
    let asked_for_match = resolve_or_open("host-agents-other").await;
    clear_if(&installed);

    assert!(
        asked_for_other.is_none(),
        "a driver must never run a different agent than the one it asked for"
    );
    assert!(asked_for_match.is_some());
}

#[tokio::test]
async fn resolve_or_open_keeps_the_lease_of_a_live_agent_for_the_whole_run() {
    let _slot = lock();
    let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let installed: Arc<dyn HostAgentResolver> = Arc::new(Leasing {
        live: "host-agents-live".into(),
        opens_as: "host-agents-live".into(),
        released: Arc::clone(&released),
    });
    install(Arc::clone(&installed));
    let opened = resolve_or_open("host-agents-live").await;
    let (_agent, lease) = opened.expect("the live agent resolves").into_parts();
    let leased_by_free_fn = resolve_leased("host-agents-live");
    clear_if(&installed);

    assert!(lease.is_some(), "a live hit carries its resolver's lease");
    assert!(leased_by_free_fn.is_some());
    assert!(
        !released.load(std::sync::atomic::Ordering::SeqCst),
        "the lease is held while the run holds it"
    );
    drop(lease);
    drop(leased_by_free_fn);
    assert!(released.load(std::sync::atomic::Ordering::SeqCst));
}
