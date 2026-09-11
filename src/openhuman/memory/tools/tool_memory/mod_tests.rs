use super::*;
use crate::openhuman::tools::traits::Tool;

#[test]
fn exports_memory_tool_wrappers_with_stable_names() {
    assert_eq!(MemoryToolsListTool.name(), "memory_tools_list");
    assert_eq!(MemoryToolsPutTool.name(), "memory_tools_put");
}
