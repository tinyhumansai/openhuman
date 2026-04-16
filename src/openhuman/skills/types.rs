//! Shared tool result types retained after QuickJS runtime removal.

use serde::{Deserialize, Serialize};

/// Result of executing a tool, containing content blocks and error status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// List of content blocks returned by the tool.
    pub content: Vec<ToolContent>,
    /// Indicates if the tool encountered an error during execution.
    #[serde(default)]
    pub is_error: bool,
}

impl ToolResult {
    pub fn success(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text { text: text.into() }],
            is_error: false,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text {
                text: message.into(),
            }],
            is_error: true,
        }
    }

    pub fn json(data: serde_json::Value) -> Self {
        Self {
            content: vec![ToolContent::Json { data }],
            is_error: false,
        }
    }

    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| match c {
                ToolContent::Text { text } => Some(text.as_str()),
                ToolContent::Json { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn output(&self) -> String {
        self.content
            .iter()
            .map(|c| match c {
                ToolContent::Text { text } => text.clone(),
                ToolContent::Json { data } => {
                    serde_json::to_string_pretty(data).unwrap_or_default()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A single content block within a `ToolResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToolContent {
    Text { text: String },
    Json { data: serde_json::Value },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_result_success() {
        let r = ToolResult::success("done");
        assert!(!r.is_error);
        assert_eq!(r.text(), "done");
        assert_eq!(r.output(), "done");
    }

    #[test]
    fn tool_result_error() {
        let r = ToolResult::error("failed");
        assert!(r.is_error);
        assert_eq!(r.text(), "failed");
    }

    #[test]
    fn tool_result_json() {
        let r = ToolResult::json(json!({"key": "value"}));
        assert!(!r.is_error);
        assert!(r.text().is_empty()); // text() skips JSON blocks
        assert!(r.output().contains("key"));
    }

    #[test]
    fn tool_result_mixed_content() {
        let r = ToolResult {
            content: vec![
                ToolContent::Text {
                    text: "line1".into(),
                },
                ToolContent::Json {
                    data: json!({"a": 1}),
                },
                ToolContent::Text {
                    text: "line2".into(),
                },
            ],
            is_error: false,
        };
        assert_eq!(r.text(), "line1\nline2");
        let output = r.output();
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
        assert!(output.contains("\"a\""));
    }

    #[test]
    fn tool_result_serde_roundtrip() {
        let r = ToolResult::success("hello");
        let json = serde_json::to_string(&r).unwrap();
        let back: ToolResult = serde_json::from_str(&json).unwrap();
        assert!(!back.is_error);
        assert_eq!(back.text(), "hello");
    }

    #[test]
    fn tool_content_text_serde() {
        let c = ToolContent::Text {
            text: "test".into(),
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        let back: ToolContent = serde_json::from_str(&json).unwrap();
        match back {
            ToolContent::Text { text } => assert_eq!(text, "test"),
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn tool_content_json_serde() {
        let c = ToolContent::Json {
            data: json!({"x": 1}),
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"type\":\"json\""));
        let back: ToolContent = serde_json::from_str(&json).unwrap();
        match back {
            ToolContent::Json { data } => assert_eq!(data["x"], 1),
            _ => panic!("expected Json variant"),
        }
    }

    #[test]
    fn tool_result_empty_content() {
        let r = ToolResult {
            content: vec![],
            is_error: false,
        };
        assert!(r.text().is_empty());
        assert!(r.output().is_empty());
    }
}
