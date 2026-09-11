mod doctor;
// `pub(crate)` (not `mod`): the tinyflows `memory` node's `OpenHumanMemory`
// adapter (`crate::openhuman::flows::tinyflows::memory_adapter`) reaches
// `flavour::lookup_flavour` / `flavour::FlavourLookup` directly so the node's
// `flavour` operation and `MemoryFlavourTool` share one flavoured-tree read
// path — see `lookup_flavour`'s doc comment.
pub(crate) mod flavour;
mod forget;
mod recall;
mod store;

// Agent tools that came back from `tinymemory-core` when the memory subsystem
// was extracted. Agent tools are host surface by the tinymemory README's split:
// they name the `Tool` trait, `ToolResult` and `ToolScope`, none of which the
// engine crate can see. Directory names track their origin inside that crate —
// `raw_store` was `store/tools/`, `search` was `search/tools/`, `tool_memory`
// was `tool_memory/tools/` — and `diff` / `goals` / `people` were each that
// domain's `tools.rs`.
pub mod goals;
pub mod raw_store;
pub mod search;
pub mod tool_memory;

pub use crate::openhuman::memory::query::*;
pub use doctor::MemoryDoctorTool;
pub use flavour::MemoryFlavourTool;
pub use forget::MemoryForgetTool;
pub use recall::MemoryRecallTool;
pub use store::MemoryStoreTool;

// The tools that came back from the extracted crate, re-exported flat so
// `openhuman::tools`'s glob keeps every historical name in scope — the
// registration sites in `tools/ops.rs` name them unqualified.
pub use goals::GoalsTool;
pub use raw_store::{MemoryStoreKindsTool, MemoryStoreRawChunksTool, MemoryStoreRawSearchTool};
pub use search::{MemoryChunkContextTool, MemoryHybridSearchTool, MemoryVectorSearchTool};
