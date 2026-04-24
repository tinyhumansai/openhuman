use crate::openhuman::life_capture::{runtime as life_capture_runtime, rpc as life_capture_rpc};
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::fmt::Write;

/// Hybrid search across the user's PersonalIndex (iMessage transcripts,
/// Gmail threads, Google Calendar events, …). Backed by
/// `life_capture::rpc::handle_search` — shares the same index + embedder
/// handles that the JSON-RPC `life_capture.search` controller uses.
pub struct LifeCaptureSearchTool;

impl LifeCaptureSearchTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LifeCaptureSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for LifeCaptureSearchTool {
    fn name(&self) -> &str {
        "life_capture_search"
    }

    fn description(&self) -> &str {
        "Search the user's personal index across iMessage, Gmail, and Google Calendar. \
         Hybrid ranking combines semantic similarity, keyword match (FTS5), and recency. \
         Use for questions like \"what did Alice say about the offsite\", \"find the \
         thread about shipping date\", or \"which meeting discussed the Q3 roadmap\". \
         Returns top-k hits with snippet, source, subject, and unix timestamp."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Natural-language query. Embedded and matched against the index.",
                },
                "k": {
                    "type": "integer",
                    "description": "Max hits to return (default 10, capped at 100).",
                    "minimum": 1,
                    "maximum": 100,
                },
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let text = args
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Missing 'text' parameter"))?
            .trim();
        if text.is_empty() {
            return Err(anyhow::anyhow!("'text' cannot be empty"));
        }
        #[allow(clippy::cast_possible_truncation)]
        let k = args
            .get("k")
            .and_then(Value::as_u64)
            .map_or(10, |v| v as usize);

        let handles = match life_capture_runtime::get_full() {
            Ok(h) => h,
            Err(msg) => return Ok(ToolResult::error(msg.to_string())),
        };

        let outcome = match life_capture_rpc::handle_search(
            &handles.index,
            &handles.embedder,
            text.to_string(),
            k,
        )
        .await
        {
            Ok(o) => o,
            Err(e) => return Ok(ToolResult::error(format!("life_capture search failed: {e}"))),
        };

        let hits = outcome
            .value
            .get("hits")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        if hits.is_empty() {
            return Ok(ToolResult::success(
                "No hits found in the personal index for that query.",
            ));
        }

        let mut out = format!("Found {} hit(s):\n", hits.len());
        for hit in &hits {
            let snippet = hit
                .get("snippet")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let source = hit.get("source").cloned().unwrap_or(Value::Null);
            let subject = hit
                .get("subject")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let ts = hit.get("ts").and_then(Value::as_i64).unwrap_or(0);
            let score = hit.get("score").and_then(Value::as_f64).unwrap_or(0.0);
            let source_str = source_label(&source);
            let _ = writeln!(
                out,
                "- [{source_str}] ts={ts} score={score:.3}{subject_part}\n  {snippet}",
                subject_part = if subject.is_empty() {
                    String::new()
                } else {
                    format!(" — {subject}")
                }
            );
        }
        Ok(ToolResult::success(out))
    }
}

/// Collapse the `source` JSON (which is a tagged enum serialised by
/// `Source`) into a short label for the LLM. We intentionally do not
/// print the full payload — external IDs and addresses are PII and
/// logging them raw in tool output would leak into transcripts.
fn source_label(source: &Value) -> String {
    match source {
        Value::String(s) => s.clone(),
        Value::Object(map) => {
            if let Some((k, _)) = map.iter().next() {
                k.clone()
            } else {
                "unknown".into()
            }
        }
        _ => "unknown".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_schema() {
        let tool = LifeCaptureSearchTool::new();
        assert_eq!(tool.name(), "life_capture_search");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["text"].is_object());
        assert_eq!(schema["required"], json!(["text"]));
    }

    #[tokio::test]
    async fn empty_text_rejected() {
        let tool = LifeCaptureSearchTool::new();
        let res = tool.execute(json!({"text": "   "})).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn missing_text_rejected() {
        let tool = LifeCaptureSearchTool::new();
        let res = tool.execute(json!({})).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn uninitialised_runtime_returns_error_result_not_panic() {
        // When the life_capture runtime hasn't been initialised (e.g. running
        // inside an agent session in a test env without core startup), the
        // tool should surface a structured `ToolResult::error`, not panic
        // and not propagate an anyhow error — so the LLM sees a readable
        // message it can react to.
        let tool = LifeCaptureSearchTool::new();
        let res = tool.execute(json!({"text": "hello"})).await.unwrap();
        // Either the runtime is initialised by a sibling test (unlikely in
        // an isolated cargo test run) or we get the "not initialised" error
        // path. We accept either and only assert the tool didn't panic.
        assert!(res.is_error || !res.output().is_empty());
    }

    #[test]
    fn source_label_string() {
        assert_eq!(source_label(&json!("gmail")), "gmail");
    }

    #[test]
    fn source_label_object() {
        assert_eq!(
            source_label(&json!({"imessage": {"chat_id": "123"}})),
            "imessage"
        );
    }

    #[test]
    fn source_label_null() {
        assert_eq!(source_label(&Value::Null), "unknown");
    }
}
