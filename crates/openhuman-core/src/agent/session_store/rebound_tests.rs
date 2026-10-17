use super::*;
use tinystoragedrivers::{MemoryStorage, Scope};

fn put_marker(stores: &ReboundSessionStores, backend: &Arc<dyn StorageBackend>) {
    // Opening an agent's stores declares its collections in the backend.
    let _ = stores.try_for_agent("probe").unwrap();
    backend.for_scope(&Scope::local()).unwrap();
}

#[test]
fn a_rebind_moves_new_agents_to_the_new_backend() {
    let first: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let second: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let registry = StoreRegistry::default();
    let stores = ReboundSessionStores::new_in(&registry, Arc::clone(&first), true).unwrap();
    put_marker(&stores, &first);
    let before = stores.destination_key();

    // Only the stores of this registry move; tests beside this one keep theirs.
    assert_eq!(registry.rebind(&second), 1);

    // The destination names the backend driver and a per-provider id; a
    // rebound store is a different provider underneath.
    assert_ne!(stores.destination_key(), before);
    stores.try_for_agent("probe").unwrap();
}

#[test]
fn handles_opened_before_a_rebind_keep_their_backend() {
    let first: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let second: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let registry = StoreRegistry::default();
    let stores = ReboundSessionStores::new_in(&registry, Arc::clone(&first), false).unwrap();
    let old = stores.try_for_agent("agent").unwrap();
    registry.rebind(&second);
    let new = stores.try_for_agent("agent").unwrap();
    old.turn_states
        .put(&tinyagents_session::turn_state::TurnState::started(
            "t",
            "r",
            4,
            "2026-01-01T00:00:00Z",
        ))
        .ok();
    assert!(
        !Arc::ptr_eq(&old.turn_states, &new.turn_states),
        "a store opened after the rebind is not the one opened before it"
    );
}

#[test]
fn dropped_stores_are_not_rebound() {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let registry = StoreRegistry::default();
    let kept = ReboundSessionStores::new_in(&registry, Arc::clone(&backend), false).unwrap();
    let dropped = ReboundSessionStores::new_in(&registry, Arc::clone(&backend), false).unwrap();
    drop(dropped);
    assert_eq!(registry.rebind(&backend), 1, "only the live store moves");
    drop(kept);
    assert_eq!(registry.rebind(&backend), 0);
}

#[test]
fn a_registry_rebind_leaves_other_registries_alone() {
    let first: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let second: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let (mine, theirs) = (StoreRegistry::default(), StoreRegistry::default());
    let moved = ReboundSessionStores::new_in(&mine, Arc::clone(&first), false).unwrap();
    let untouched = ReboundSessionStores::new_in(&theirs, Arc::clone(&first), false).unwrap();
    let before = untouched.destination_key();

    assert_eq!(mine.rebind(&second), 1);

    assert_eq!(untouched.destination_key(), before);
    drop(moved);
}

#[test]
fn the_provider_surface_forwards_to_the_current_backend() {
    use tinyagents_harness::store::SchemaAdmission;

    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let registry = StoreRegistry::default();
    let stores = ReboundSessionStores::new_in(&registry, Arc::clone(&backend), false).unwrap();

    // Debug names the live destination rather than the wrapper's internals.
    let rendered = format!("{stores:?}");
    assert!(rendered.contains("ReboundSessionStores"), "{rendered}");
    assert!(rendered.contains("destination"), "{rendered}");

    // `for_agent` and `try_for_agent` open the same agent scope: a turn state
    // written through one is read back through the other.
    let infallible = stores.for_agent("forwarded");
    let fallible = stores.try_for_agent("forwarded").unwrap();
    infallible
        .turn_states
        .put(&tinyagents_session::turn_state::TurnState::started(
            "t-fwd",
            "r",
            4,
            "2026-01-01T00:00:00Z",
        ))
        .unwrap();
    assert!(fallible.turn_states.get("t-fwd").unwrap().is_some());

    assert!(stores.recover().is_ok());
    assert_eq!(
        stores.destination_key(),
        stores.current().destination_key(),
        "the destination is the current inner provider's"
    );
    stores
        .run_ledger_for_agent("forwarded")
        .expect("the ledger lookup does not fail on a fresh memory backend");
    stores
        .conversations_for_agent("forwarded")
        .expect("the conversation lookup does not fail on a fresh memory backend");

    // Binding an actor goes through the inner provider, with and without an
    // explicit schema admission mode; both agree on whether it binds.
    let other: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let plain = stores.bind_actor(Arc::clone(&other)).unwrap().is_some();
    let admitted = stores
        .bind_actor_with_schema_admission(other, SchemaAdmission::Declare)
        .unwrap()
        .is_some();
    assert_eq!(plain, admitted);
}
