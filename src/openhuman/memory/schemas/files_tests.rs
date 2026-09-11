use super::*;

#[test]
fn file_schema_exposes_all_functions() {
    assert_eq!(FUNCTIONS, &["list_files", "read_file", "write_file"]);
    assert_eq!(controllers().len(), FUNCTIONS.len());
}

#[test]
fn unknown_file_schema_returns_none() {
    assert!(schema("not_real").is_none());
}

#[test]
fn write_file_schema_requires_path_and_content() {
    let schema = schema("write_file").unwrap();
    let required: Vec<&str> = schema
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert!(required.contains(&"relative_path"));
    assert!(required.contains(&"content"));
}
