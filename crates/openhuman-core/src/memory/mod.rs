//! Memory v2: recall, fetch and store over a pluggable engine
//! (`docs/specs/memory-v2.md`; contract in `vendor/tinymemory`).
//!
//! | Module | Role |
//! | --- | --- |
//! | [`engine`] | Binds the `[memory]` engine (`tinyhumans` over the host's backend credential, or `cortexdb` with a stored key); memory is **off** without one |
//! | [`ops`] | Engine selection, recall, fetch, learn, forget, list; [`ops::store_item`] scrubs before storing |
//! | [`conversations`] | Per-thread batching of committed turns into `Conversation` items |
//! | [`sources`] | The `[[memory.sources]]` registry and sync (folder, file, link, github, rss, composio) |
//! | [`context`] | `context.md`: compile, read, and the new-session injection block |
//! | [`import`] | Consent-gated, resumable import of a v1 store |
//! | [`tools`] | The single `memory` agent tool |
//! | [`bus`] | Ingest + cron subscribers and the idle flusher |
//! | [`schemas`] | The `openhuman.memory_*` controllers |
//!
//! Chat thread persistence is not memory: it lives in [`crate::threads::store`].

pub mod bus;
pub mod context;
pub mod conversations;
pub mod engine;
pub mod error;
pub mod exit;
pub mod import;
pub mod ops;
pub mod schemas;
pub mod sources;
pub mod status;
pub mod tools;
pub mod types;

#[cfg(test)]
pub(crate) mod test_fixtures;

pub use bus::register_memory_subscribers;
pub use engine::is_on as memory_is_on;
pub use error::{MemoryError, MemoryResult};
pub use schemas::{
    all_controller_schemas as all_memory_controller_schemas,
    all_registered_controllers as all_memory_registered_controllers,
};
pub use tools::{MemoryTool, MEMORY_TOOL_NAME};
