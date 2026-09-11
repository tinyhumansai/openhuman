use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

fn test_security(workspace: std::path::PathBuf) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        action_dir: workspace.clone(),
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
