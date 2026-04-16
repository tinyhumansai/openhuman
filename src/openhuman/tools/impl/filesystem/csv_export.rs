use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// Export structured data (JSON array of objects) as a CSV file to the workspace.
pub struct CsvExportTool {
    security: Arc<SecurityPolicy>,
}

impl CsvExportTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

/// Escape a value for inclusion in a CSV cell. Wraps the value in
/// double-quotes when it contains commas, quotes, or newlines. Embedded
/// double-quotes are escaped by doubling them per RFC 4180.
fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        let escaped = value.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        value.to_string()
    }
}

/// Convert a `serde_json::Value` into a plain string suitable for a CSV
/// cell. Objects and arrays are serialised as compact JSON; booleans and
/// numbers use their natural representation; nulls become the empty
/// string.
fn value_to_cell(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        // Nested objects/arrays → compact JSON string
        other => other.to_string(),
    }
}

/// Collect column headers from a JSON array. If `columns` is provided,
/// use those in order. Otherwise, collect all keys from the first object
/// in the array (sorted alphabetically — serde_json uses BTreeMap by
/// default). Callers who need a specific column order should pass the
/// `columns` parameter.
fn resolve_columns(items: &[serde_json::Value], columns: Option<&[String]>) -> Vec<String> {
    if let Some(cols) = columns {
        return cols.to_vec();
    }
    // Collect keys from the first object.
    if let Some(first) = items.first() {
        if let Some(obj) = first.as_object() {
            return obj.keys().cloned().collect();
        }
    }
    Vec::new()
}

/// Render a JSON array of objects into a CSV string.
fn render_csv(items: &[serde_json::Value], columns: &[String]) -> String {
    let mut buf = String::new();

    // Header row
    let header: Vec<String> = columns.iter().map(|c| csv_escape(c)).collect();
    buf.push_str(&header.join(","));
    buf.push('\n');

    // Data rows
    for item in items {
        let row: Vec<String> = columns
            .iter()
            .map(|col| {
                let cell_value = item.get(col).map(value_to_cell).unwrap_or_default();
                csv_escape(&cell_value)
            })
            .collect();
        buf.push_str(&row.join(","));
        buf.push('\n');
    }

    buf
}

#[async_trait]
impl Tool for CsvExportTool {
    fn name(&self) -> &str {
        "csv_export"
    }

    fn description(&self) -> &str {
        "Export structured data (JSON array of objects) as a CSV file to the workspace. \
         Returns the file path. Use when the user wants raw tabular data from a tool \
         result that's too large to include inline."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "data": {
                    "type": "string",
                    "description": "JSON string containing an array of objects to export. Each object becomes a row; keys become column headers."
                },
                "filename": {
                    "type": "string",
                    "description": "Output filename (without path). Will be written to workspace/exports/. Example: 'github-issues-2026-04-16.csv'"
                },
                "columns": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional ordered list of column names to include. If omitted, all keys from the first object are used as headers."
                }
            },
            "required": ["data", "filename"]
        })
    }

    fn permission_level(&self) -> crate::openhuman::tools::traits::PermissionLevel {
        crate::openhuman::tools::traits::PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let data_str = args
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'data' parameter"))?;

        let filename = args
            .get("filename")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'filename' parameter"))?;

        let columns: Option<Vec<String>> = args.get("columns").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(String::from))
                    .collect()
            })
        });

        // Security: check write permission
        if !self.security.can_act() {
            return Ok(ToolResult::error("Action blocked: autonomy is read-only"));
        }

        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }

        // Parse the JSON data
        let parsed: serde_json::Value = match serde_json::from_str(data_str) {
            Ok(v) => v,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Failed to parse data as JSON: {e}"
                )));
            }
        };

        let items = match parsed.as_array() {
            Some(arr) => arr,
            None => {
                return Ok(ToolResult::error(
                    "Data must be a JSON array of objects, but got a non-array value",
                ));
            }
        };

        if items.is_empty() {
            return Ok(ToolResult::error("Data array is empty — nothing to export"));
        }

        // Resolve columns and render CSV
        let cols = resolve_columns(items, columns.as_deref());
        let csv_content = render_csv(items, &cols);
        let csv_bytes = csv_content.len();

        // Validate the relative path
        let relative_path = format!("exports/{filename}");
        if !self.security.is_path_allowed(&relative_path) {
            return Ok(ToolResult::error(format!(
                "Path not allowed by security policy: {relative_path}"
            )));
        }

        let full_path = self.security.workspace_dir.join(&relative_path);

        let Some(parent) = full_path.parent() else {
            return Ok(ToolResult::error("Invalid path: missing parent directory"));
        };

        // Ensure exports/ directory exists
        tokio::fs::create_dir_all(parent).await?;

        // Resolve parent AFTER creation to block symlink escapes.
        let resolved_parent = match tokio::fs::canonicalize(parent).await {
            Ok(p) => p,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Failed to resolve file path: {e}"
                )));
            }
        };

        if !self.security.is_resolved_path_allowed(&resolved_parent) {
            return Ok(ToolResult::error(format!(
                "Resolved path escapes workspace: {}",
                resolved_parent.display()
            )));
        }

        let Some(file_name) = full_path.file_name() else {
            return Ok(ToolResult::error("Invalid path: missing file name"));
        };

        let resolved_target = resolved_parent.join(file_name);

        // If the target already exists and is a symlink, refuse to follow it
        if let Ok(meta) = tokio::fs::symlink_metadata(&resolved_target).await {
            if meta.file_type().is_symlink() {
                return Ok(ToolResult::error(format!(
                    "Refusing to write through symlink: {}",
                    resolved_target.display()
                )));
            }
        }

        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        // Write the CSV file
        match tokio::fs::write(&resolved_target, &csv_content).await {
            Ok(()) => {
                let size_display = if csv_bytes >= 1024 * 1024 {
                    format!("{:.1} MB", csv_bytes as f64 / (1024.0 * 1024.0))
                } else if csv_bytes >= 1024 {
                    format!("{:.1} KB", csv_bytes as f64 / 1024.0)
                } else {
                    format!("{csv_bytes} bytes")
                };

                Ok(ToolResult::success(format!(
                    "Exported {} rows to {relative_path} ({size_display})",
                    items.len()
                )))
            }
            Err(e) => Ok(ToolResult::error(format!("Failed to write CSV file: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

    fn test_security(workspace: std::path::PathBuf) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: workspace,
            ..SecurityPolicy::default()
        })
    }

    #[test]
    fn csv_export_name() {
        let tool = CsvExportTool::new(test_security(std::env::temp_dir()));
        assert_eq!(tool.name(), "csv_export");
    }

    #[test]
    fn csv_export_schema_has_required_fields() {
        let tool = CsvExportTool::new(test_security(std::env::temp_dir()));
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["data"].is_object());
        assert!(schema["properties"]["filename"].is_object());
        assert!(schema["properties"]["columns"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("data")));
        assert!(required.contains(&json!("filename")));
    }

    #[tokio::test]
    async fn csv_export_formats_simple_array() {
        let dir = std::env::temp_dir().join("openhuman_test_csv_export_simple");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = CsvExportTool::new(test_security(dir.clone()));
        let data = serde_json::to_string(&json!([
            {"name": "Alice", "age": 30, "city": "NYC"},
            {"name": "Bob", "age": 25, "city": "LA"},
            {"name": "Carol", "age": 35, "city": "Chicago"}
        ]))
        .unwrap();

        let result = tool
            .execute(json!({
                "data": data,
                "filename": "people.csv"
            }))
            .await
            .unwrap();

        assert!(!result.is_error, "unexpected error: {}", result.output());
        assert!(result.output().contains("3 rows"));

        let content = tokio::fs::read_to_string(dir.join("exports/people.csv"))
            .await
            .unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 4, "header + 3 data rows");

        // Header should contain the keys from the first object
        let header = lines[0];
        assert!(header.contains("name"));
        assert!(header.contains("age"));
        assert!(header.contains("city"));

        // Data rows should contain values
        assert!(lines[1].contains("Alice"));
        assert!(lines[2].contains("Bob"));
        assert!(lines[3].contains("Carol"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn csv_export_handles_missing_keys() {
        let dir = std::env::temp_dir().join("openhuman_test_csv_export_missing_keys");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = CsvExportTool::new(test_security(dir.clone()));
        let data = serde_json::to_string(&json!([
            {"name": "Alice", "age": 30, "city": "NYC"},
            {"name": "Bob"},
            {"name": "Carol", "city": "Chicago"}
        ]))
        .unwrap();

        let result = tool
            .execute(json!({
                "data": data,
                "filename": "sparse.csv",
                "columns": ["name", "age", "city"]
            }))
            .await
            .unwrap();

        assert!(!result.is_error, "unexpected error: {}", result.output());

        let content = tokio::fs::read_to_string(dir.join("exports/sparse.csv"))
            .await
            .unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 4);

        // Bob's row should have empty cells for age and city
        let bob_row = lines[2];
        let bob_cells: Vec<&str> = bob_row.split(',').collect();
        assert_eq!(bob_cells.len(), 3, "Bob row should have 3 cells");
        assert_eq!(bob_cells[0], "Bob");
        assert_eq!(bob_cells[1], "", "missing age should be empty");
        assert_eq!(bob_cells[2], "", "missing city should be empty");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn csv_export_respects_column_order() {
        let dir = std::env::temp_dir().join("openhuman_test_csv_export_column_order");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = CsvExportTool::new(test_security(dir.clone()));
        let data = serde_json::to_string(&json!([
            {"name": "Alice", "age": 30, "city": "NYC"},
            {"name": "Bob", "age": 25, "city": "LA"}
        ]))
        .unwrap();

        let result = tool
            .execute(json!({
                "data": data,
                "filename": "ordered.csv",
                "columns": ["city", "name", "age"]
            }))
            .await
            .unwrap();

        assert!(!result.is_error, "unexpected error: {}", result.output());

        let content = tokio::fs::read_to_string(dir.join("exports/ordered.csv"))
            .await
            .unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(
            lines[0], "city,name,age",
            "header must follow requested column order"
        );
        assert_eq!(lines[1], "NYC,Alice,30");
        assert_eq!(lines[2], "LA,Bob,25");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn csv_export_rejects_non_array_input() {
        let dir = std::env::temp_dir().join("openhuman_test_csv_export_non_array");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = CsvExportTool::new(test_security(dir.clone()));
        let data = serde_json::to_string(&json!({"not": "an array"})).unwrap();

        let result = tool
            .execute(json!({
                "data": data,
                "filename": "bad.csv"
            }))
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(
            result.output().contains("non-array"),
            "error should mention non-array, got: {}",
            result.output()
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn csv_export_handles_nested_values() {
        let dir = std::env::temp_dir().join("openhuman_test_csv_export_nested");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = CsvExportTool::new(test_security(dir.clone()));
        let data = serde_json::to_string(&json!([
            {
                "name": "Alice",
                "tags": ["admin", "dev"],
                "meta": {"role": "lead", "level": 5}
            },
            {
                "name": "Bob",
                "tags": [],
                "meta": null
            }
        ]))
        .unwrap();

        let result = tool
            .execute(json!({
                "data": data,
                "filename": "nested.csv",
                "columns": ["name", "tags", "meta"]
            }))
            .await
            .unwrap();

        assert!(!result.is_error, "unexpected error: {}", result.output());

        let content = tokio::fs::read_to_string(dir.join("exports/nested.csv"))
            .await
            .unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 3, "header + 2 data rows");

        // Alice's tags should be serialized as a JSON string (in quotes because it contains commas)
        let alice_row = lines[1];
        assert!(alice_row.contains("Alice"));
        // The JSON array should be serialized as a string and quoted
        assert!(
            alice_row.contains(r#"[""admin"",""dev""]"#),
            "nested arrays should be JSON-serialized in CSV: {alice_row}"
        );

        // Bob's meta is null → empty cell
        let bob_row = lines[2];
        assert!(bob_row.contains("Bob"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
