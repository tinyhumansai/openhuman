use super::*;
use serde_json::{json, Value};

/// The persisted catalogue is recorded in session transcripts
/// (`session_host/recorded_tools.rs`), so every tool's name, description,
/// parameter schema, permission and external-effect flag must stay
/// byte-identical across moves of the code behind it. Regenerate the snapshot
/// deliberately with `UPDATE_FLOW_TOOL_SNAPSHOT=1`.
fn catalogue() -> Value {
    let config = Arc::new(Config::default());
    let security = Arc::new(SecurityPolicy::default());
    Value::Array(
        tools(config, &security)
            .iter()
            .map(|t| {
                json!({
                    "name": t.name(),
                    "description": t.description(),
                    "parameters": t.parameters_schema(),
                    "permission": format!("{:?}", t.permission_level()),
                    "external_effect": t.external_effect(),
                })
            })
            .collect(),
    )
}

#[test]
fn flow_tool_catalogue_matches_recorded_snapshot() {
    let actual = catalogue();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/flows/registration_tests/flow_tools.snap.json");
    if std::env::var_os("UPDATE_FLOW_TOOL_SNAPSHOT").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&actual).unwrap() + "\n").unwrap();
    }
    let expected: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        actual, expected,
        "flow tool catalogue drifted from registration_tests/flow_tools.snap.json"
    );
}
