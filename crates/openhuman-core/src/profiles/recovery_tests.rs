use super::*;
use crate::core::runtime::{ContextOverlay, DomainSet};
use std::sync::Arc;
use tinyagents_session::port::SessionStoreProvider;
use tinyagents_session::turn_state::{TurnLifecycle, TurnState};
use tinystoragedrivers::{MemoryStorage, StorageBackend};

fn profile() -> ProfileId {
    ProfileId::for_user("recovering", crate::profiles::ProfileIdMode::Raw).unwrap()
}

/// A context for the profile whose host overrides carry a storage-backed
/// session store (no workspace files), as a SaaS host installs.
fn context_with_store() -> (Arc<CoreContext>, Arc<dyn SessionStoreProvider>) {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let provider: Arc<dyn SessionStoreProvider> = Arc::new(
        tinyagents_session::DriverSessionStores::new(backend)
            .unwrap()
            .recover_on_open(false),
    );
    let mut overrides = crate::agent::host_overrides::HostOverrides::default();
    overrides.session_store = Some(Arc::clone(&provider));
    let overlay = ContextOverlay::new(
        crate::config::Config::default(),
        DomainSet::kernel(),
        crate::tools::toolpacks::ToolGroups::none(),
    )
    .profile("recovering")
    .host_overrides(Arc::new(overrides));
    (
        CoreContext::for_test(DomainSet::full(), None).derive_with(overlay),
        provider,
    )
}

#[test]
fn without_an_installed_store_the_session_sweep_does_nothing() {
    let (context, _) = context_with_store();
    // No store is in effect on this thread, so there is nothing to sweep.
    recover_session_store(&profile(), &context);
}

#[test]
fn a_stored_turn_left_mid_flight_is_marked_interrupted() {
    let (context, provider) = context_with_store();
    let key = crate::core::runtime::session_key(&crate::core::runtime::Tenant::of(&context));
    let turn_states = provider.for_agent(&key).turn_states;
    turn_states
        .put(&TurnState::started(
            "thread-1",
            "req-1",
            4,
            "2026-10-09T00:00:00Z",
        ))
        .unwrap();

    CoreContext::sync_scope(Arc::clone(&context), || {
        recover_session_store(&profile(), &context);
    });

    let after = turn_states.get("thread-1").unwrap().unwrap();
    assert_eq!(after.lifecycle, TurnLifecycle::Interrupted);
}

#[test]
fn a_second_sweep_finds_nothing_left_to_recover() {
    let (context, provider) = context_with_store();
    let key = crate::core::runtime::session_key(&crate::core::runtime::Tenant::of(&context));
    let turn_states = provider.for_agent(&key).turn_states;
    turn_states
        .put(&TurnState::started(
            "thread-2",
            "req-2",
            4,
            "2026-10-09T00:00:00Z",
        ))
        .unwrap();
    CoreContext::sync_scope(Arc::clone(&context), || {
        recover_session_store(&profile(), &context);
        recover_session_store(&profile(), &context);
    });
    assert_eq!(
        turn_states.get("thread-2").unwrap().unwrap().lifecycle,
        TurnLifecycle::Interrupted
    );
}
