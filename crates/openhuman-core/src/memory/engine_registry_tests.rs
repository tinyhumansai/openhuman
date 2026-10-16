use super::*;
use crate::memory::billing::free_period_active;
use crate::memory::engine::{bind_with_root, resolve};
use crate::memory::test_fixtures::config_in;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tinymemory_api::conformance::ReferenceEngine;

const ID: &str = "registry-test-engine";

fn config_for(tmp: &tempfile::TempDir) -> Config {
    let mut config = config_in(tmp);
    config.memory.engine = ID.to_string();
    config
}

fn entry(roots: Arc<Mutex<Vec<Option<String>>>>, free: Option<FreeBackground>) -> EngineEntry {
    EngineEntry {
        build: Arc::new(move |request| {
            roots.lock().unwrap().push(request.root.map(str::to_string));
            Binding::On(bound_engine(
                ID,
                "registered://engine",
                Arc::new(ReferenceEngine::new()),
            ))
        }),
        free_background: free,
    }
}

#[test]
fn an_unregistered_engine_id_stays_off() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_for(&tmp);
    assert!(!resolve(&config).is_on());
}

#[test]
fn a_registered_engine_is_bound_with_its_endpoint() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_for(&tmp);
    register_memory_engine(ID, entry(Arc::default(), None));
    let binding = resolve(&config);
    unregister_memory_engine(ID);
    let bound = binding.engine().expect("bound");
    assert_eq!(bound.id, ID);
    assert_eq!(bound.endpoint, "registered://engine");
}

#[test]
fn the_layout_root_reaches_the_registered_builder() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_for(&tmp);
    let roots = Arc::new(Mutex::new(Vec::new()));
    register_memory_engine(ID, entry(Arc::clone(&roots), None));
    let _ = bind_with_root(&config, Some("user:abc"));
    let _ = bind_with_root(&config, None);
    unregister_memory_engine(ID);
    assert_eq!(
        *roots.lock().unwrap(),
        vec![Some("user:abc".to_string()), None]
    );
}

#[tokio::test]
async fn a_registered_engine_without_a_predicate_is_free() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_for(&tmp);
    register_memory_engine(ID, entry(Arc::default(), None));
    let free = free_period_active(&config).await;
    unregister_memory_engine(ID);
    assert!(free);
}

#[tokio::test]
async fn the_registered_predicate_decides_whether_background_work_is_free() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_for(&tmp);
    let answer = Arc::new(AtomicBool::new(false));
    let shared = Arc::clone(&answer);
    let predicate: FreeBackground = Arc::new(move |_, bound| {
        let shared = Arc::clone(&shared);
        Box::pin(async move {
            assert_eq!(bound.id, ID);
            shared.load(Ordering::SeqCst)
        })
    });
    register_memory_engine(ID, entry(Arc::default(), Some(predicate)));
    let paused = free_period_active(&config).await;
    answer.store(true, Ordering::SeqCst);
    let free = free_period_active(&config).await;
    unregister_memory_engine(ID);
    assert!(!paused);
    assert!(free);
}

#[tokio::test]
async fn an_unbound_engine_is_not_free() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_for(&tmp);
    assert!(!free_period_active(&config).await);
}
