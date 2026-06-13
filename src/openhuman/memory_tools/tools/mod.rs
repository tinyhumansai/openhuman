//! Agent tools for reading and writing tool-scoped memory.
//!
//! The agent uses these to introspect what rules / learnings exist for a
//! specific tool and to record new ones discovered mid-session. They are
//! the user-facing read/write surface on top of [`ToolMemoryStore`].

mod list;
mod put;

pub use list::MemoryToolsListTool;
pub use put::MemoryToolsPutTool;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::Tool;

    #[test]
    fn exports_memory_tool_wrappers_with_stable_names() {
        assert_eq!(MemoryToolsListTool.name(), "memory_tools_list");
        assert_eq!(MemoryToolsPutTool.name(), "memory_tools_put");
    }
}
