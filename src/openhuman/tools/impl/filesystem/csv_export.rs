use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tinytools::ToolRunContext;

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
        self.execute_in_context(args, None).await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.execute_in_context(args, context).await
    }
}

impl CsvExportTool {
    async fn execute_in_context(
        &self,
        args: serde_json::Value,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
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
            return Ok(ToolResult::error(
                "[policy-blocked] Action blocked: autonomy is read-only",
            ));
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

        let path_policy = super::security_for_tool_context(&self.security, context, "csv_export");

        // Security check first: validate path string, resolve symlinks, confirm workspace
        // containment. validate_parent_path walks up to the deepest existing ancestor so
        // it does not require the exports/ directory to exist yet.
        let resolved_target = match path_policy.validate_parent_path(&relative_path).await {
            Ok(p) => p,
            Err(msg) => return Ok(ToolResult::error(msg)),
        };

        // Create exports/ directory only at the validated, resolved location.
        if let Some(resolved_parent) = resolved_target.parent() {
            tokio::fs::create_dir_all(resolved_parent).await?;
        }

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
#[path = "csv_export_tests.rs"]
mod tests;
