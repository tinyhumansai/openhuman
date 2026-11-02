use super::*;
use std::sync::Arc;

use openhuman_core::core::runtime::{DomainSet, ServiceSet, TokenSource};
use openhuman_core::core::types::HostKind;

fn configured() -> RuntimeBuilder {
    let mut domains = DomainSet::full();
    domains.mcp = false;
    RuntimeBuilder::cli()
        .domains(domains)
        .services(ServiceSet::headless_api())
        .token(TokenSource::Fixed(Arc::new("bearer".into())))
        .listen_host("0.0.0.0")
        .listen_port(7811)
}

#[test]
fn split_for_cli_keeps_the_runtime_knobs_on_the_builder() {
    let before = configured().summary();
    let (builder, _globals) = configured().split_for_cli();
    let after = builder.summary();
    assert_eq!(after.host_kind, before.host_kind);
    assert_eq!(after.domains, before.domains);
    assert_eq!(after.domains.map(|d| d.mcp), Some(false));
    assert_eq!(after.services, Some(ServiceSet::headless_api()));
    assert!(after.fixed_token, "the bearer is not dropped");
    assert_eq!(after.listen_host.as_deref(), Some("0.0.0.0"));
    assert_eq!(after.listen_port, Some(7811));
    assert_eq!(after.workspace_dir, before.workspace_dir);
    assert_eq!(after.config_source, before.config_source);
}

#[test]
fn split_for_cli_moves_process_globals_out_and_drops_live_policy() {
    let mut builder = configured();
    builder = builder.server_launcher(|_| Box::pin(async { Ok(()) }));
    let (rest, globals) = builder.split_for_cli();
    assert!(globals.seams.server_launcher.is_some());
    let summary = rest.summary();
    assert!(!summary.has_server_launcher, "installed once, not twice");
    assert!(!summary.has_live_policy);
}

#[test]
fn the_launcher_takes_the_handed_over_builder_once() {
    let (builder, _globals) = configured().split_for_cli();
    let boot = HostBoot::new(builder);
    let taken = RuntimeBuilder::from_host_boot(&boot).expect("the builder rides the boot");
    assert_eq!(taken.summary().listen_port, Some(7811));
    assert_eq!(taken.summary().host_kind, HostKind::detect_standalone());
    assert!(RuntimeBuilder::from_host_boot(&boot).is_none());
}

#[test]
fn split_for_cli_keeps_the_session_store_for_a_stateless_workspace() {
    use tinyagents_session::port::InMemorySessionStores;
    let store: Arc<dyn SessionStoreProvider> = Arc::new(InMemorySessionStores::new());
    let builder = configured()
        .workspace(crate::Workspace::Stateless)
        .session_store(store);
    let (rest, globals) = builder.split_for_cli();
    assert!(
        globals.session_store.is_some(),
        "installed as a process global"
    );
    assert!(
        rest.summary().has_session_store,
        "and still on the builder, so `build()` accepts the stateless workspace"
    );
}

#[tokio::test]
async fn split_for_cli_keeps_the_storage_seam_on_both_sides() {
    let backend = openhuman_core::storage::open("memory").await.unwrap();
    let (rest, globals) = configured()
        .storage(StorageSource::Backend(backend))
        .split_for_cli();
    assert!(matches!(
        globals.seams.storage,
        Some(StorageSource::Backend(_))
    ));
    assert!(matches!(
        rest.seams.storage,
        Some(StorageSource::Backend(_))
    ));
}
