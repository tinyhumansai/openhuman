use std::sync::Arc;

use crate::openhuman::memory::Memory;

use tinycortex::memory::tool_memory::store::ToolMemoryStore;

/// Build the crate-owned store over OpenHuman's shared memory object.
pub fn tool_memory_store(memory: Arc<dyn Memory>) -> ToolMemoryStore {
    ToolMemoryStore::new(memory)
}
