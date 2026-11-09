use std::sync::Arc;

use tinymemory_api::{MemoryEngine, StoreItem};
use tinymemory_integrations::safety::{scrub_item_with, Policy};
use tinymemory_integrations::EngineBinder;

fn wrap(engine: Arc<dyn MemoryEngine>) -> Arc<dyn MemoryEngine> {
    EngineBinder::default()
        .bind_external(engine, "test://engine", Policy::corroborated())
        .engine
}

fn scrub(item: StoreItem) -> StoreItem {
    scrub_item_with(item, Policy::corroborated()).value
}

#[path = "guard_inner_tests.rs"]
mod tests;
