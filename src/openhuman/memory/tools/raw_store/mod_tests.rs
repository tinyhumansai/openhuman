use super::*;
use crate::openhuman::tools::traits::Tool;

#[test]
fn exports_memory_store_tools_with_stable_names() {
    assert_eq!(MemoryStoreKindsTool.name(), "memory_store_kinds");
    assert_eq!(MemoryStoreRawChunksTool.name(), "memory_store_raw_chunks");
    assert_eq!(MemoryStoreRawSearchTool.name(), "memory_store_raw_search");
}
