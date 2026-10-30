//! Closed argument contract for source-evidence operations.

use serde_json::Value;

use super::{MAX_FILES, MAX_FILE_BYTES, MAX_VISITED_ENTRIES};

/// Validate host arguments before resolving paths. Optional omitted keys retain
/// their documented defaults; explicitly malformed keys must never turn a
/// narrowly targeted operation into a full workspace scan.
pub(super) fn validate_source_args(args: &Value) -> anyhow::Result<()> {
    let Some(map) = args.as_object() else {
        anyhow::bail!("source evidence arguments must be a JSON object");
    };
    // This is an opt-in data-exposure tool: a typo in "operation" or "path"
    // must not silently fall back to scanning every eligible workspace file.
    // The same closed key set is advertised in the JSON schema.
    for key in map.keys() {
        if !matches!(
            key.as_str(),
            "operation"
                | "path"
                | "query"
                | "max_files"
                | "max_entries"
                | "after_path"
                | "max_edges"
                | "max_results"
                | "focus_path"
                | "expected_sha256"
                | "start_line"
                | "end_line"
                | "start_column"
        ) {
            anyhow::bail!("unrecognized source evidence argument");
        }
    }
    let operation = match map.get("operation") {
        None => "scan",
        Some(Value::String(op)) => op.as_str(),
        Some(_) => anyhow::bail!("operation must be a string"),
    };
    if !matches!(operation, "scan" | "locate" | "graph" | "verify") {
        anyhow::bail!("operation must be scan, locate, graph, or verify");
    }
    for key in [
        "path",
        "query",
        "after_path",
        "focus_path",
        "expected_sha256",
    ] {
        if let Some(value) = map.get(key) {
            if !value.is_string() {
                anyhow::bail!("{key} must be a string when provided");
            }
        }
    }
    if map.get("path").and_then(Value::as_str) == Some("") {
        anyhow::bail!("path cannot be an empty string; omit it to use the workspace");
    }
    for (key, maximum) in [
        ("path", 4096usize),
        ("query", 128),
        ("after_path", 180),
        ("focus_path", 180),
    ] {
        if let Some(value) = map.get(key).and_then(Value::as_str) {
            if value.len() > maximum {
                anyhow::bail!("{key} exceeds the {maximum}-byte limit");
            }
        }
    }
    if let Some(digest) = map.get("expected_sha256").and_then(Value::as_str) {
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            anyhow::bail!("expected_sha256 must be 64 hexadecimal characters");
        }
    }
    if map
        .get("focus_path")
        .and_then(Value::as_str)
        .is_some_and(|value| value.chars().any(char::is_control))
    {
        anyhow::bail!("focus_path contains control characters");
    }
    for (key, maximum) in [
        ("max_files", MAX_FILES as u64),
        ("max_entries", MAX_VISITED_ENTRIES as u64),
        ("max_edges", 64),
        ("max_results", 5),
        ("start_column", MAX_FILE_BYTES + 1),
        ("start_line", u64::MAX),
        ("end_line", u64::MAX),
    ] {
        if let Some(value) = map.get(key) {
            let Some(number) = value.as_u64() else {
                anyhow::bail!("{key} must be a positive integer");
            };
            if number == 0 || number > maximum {
                anyhow::bail!("{key} must be between 1 and {maximum}");
            }
        }
    }
    if operation == "verify" && map.get("path").is_none() {
        anyhow::bail!("verify requires an explicit source file path");
    }
    Ok(())
}
