//! Re-export from `agent_memory::memory_loader` — canonical home moved in
//! the `agent_memory` domain consolidation. Existing `use` paths in the
//! harness continue to work via this facade.

pub use crate::openhuman::agent_memory::memory_loader::*;
