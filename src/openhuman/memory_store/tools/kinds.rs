//! `memory_store_kinds` — introspection. Enumerate every supported
//! [`MemoryKind`] so an agent can plan a fan-out without hard-coding.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::memory_store::MemoryKind;
use crate::openhuman::tools::traits::{Tool, ToolResult};

pub struct MemoryStoreKindsTool;

#[async_trait]
impl Tool for MemoryStoreKindsTool {
    fn name(&self) -> &str {
        "memory_store_kinds"
    }

    fn description(&self) -> &str {
        "Return the catalog of memory_store storage kinds (content, chunk, \
         tree, vector, document, kv, graph, contact). No arguments. Use \
         when planning a multi-kind retrieval fan-out."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][memory_store] kinds start");
        let kinds: Vec<&'static str> = MemoryKind::ALL.iter().map(|k| k.as_str()).collect();
        let json = serde_json::to_string(&json!({ "kinds": kinds }))?;
        log::debug!(
            "[tool][memory_store] kinds success count={}",
            MemoryKind::ALL.len()
        );
        Ok(ToolResult::success(json))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_schema_is_empty_object() {
        let tool = MemoryStoreKindsTool;
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"], json!({}));
    }

    #[tokio::test]
    async fn execute_returns_all_memory_kinds() {
        let tool = MemoryStoreKindsTool;
        let result = tool.execute(Value::Null).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
        let expected: Vec<&str> = MemoryKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(parsed["kinds"], json!(expected));
    }
}
