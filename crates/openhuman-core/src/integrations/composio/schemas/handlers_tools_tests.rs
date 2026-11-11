use super::*;
use serde_json::json;

#[test]
fn execute_parser_trims_tool_preserves_optional_values_and_ignores_unknown_keys() {
    let params = Map::from_iter([
        ("tool".into(), json!("  GMAIL_SEND_EMAIL  ")),
        ("arguments".into(), json!(42)),
        ("connection_id".into(), json!(" conn-1 ")),
        ("unrecognized".into(), json!(true)),
    ]);

    assert_eq!(
        parse_execute(params).unwrap(),
        ExecuteParams {
            tool: "GMAIL_SEND_EMAIL".into(),
            arguments: Some(json!(42)),
            connection_id: Some(" conn-1 ".into()),
        }
    );

    let blank_connection = Map::from_iter([
        ("tool".into(), json!("GMAIL_SEND_EMAIL")),
        ("connection_id".into(), json!("")),
    ]);
    assert_eq!(
        parse_execute(blank_connection)
            .unwrap()
            .connection_id
            .as_deref(),
        Some("")
    );
}

#[test]
fn execute_parser_defaults_missing_and_null_optional_fields_to_none() {
    let missing = Map::from_iter([("tool".into(), json!("GMAIL_SEND_EMAIL"))]);
    assert_eq!(
        parse_execute(missing).unwrap(),
        ExecuteParams {
            tool: "GMAIL_SEND_EMAIL".into(),
            arguments: None,
            connection_id: None,
        }
    );

    let nulls = Map::from_iter([
        ("tool".into(), json!("GMAIL_SEND_EMAIL")),
        ("arguments".into(), Value::Null),
        ("connection_id".into(), Value::Null),
    ]);
    assert_eq!(
        parse_execute(nulls).unwrap(),
        ExecuteParams {
            tool: "GMAIL_SEND_EMAIL".into(),
            arguments: None,
            connection_id: None,
        }
    );
}

#[test]
fn execute_parser_preserves_tool_first_errors_and_connection_type_error() {
    let missing_tool = Map::from_iter([
        ("arguments".into(), json!("bad-but-json")),
        ("connection_id".into(), json!(7)),
    ]);
    assert_eq!(
        parse_execute(missing_tool).unwrap_err(),
        "missing required param 'tool'"
    );

    let invalid_tool = Map::from_iter([
        ("tool".into(), json!(42)),
        ("connection_id".into(), json!(7)),
    ]);
    assert!(parse_execute(invalid_tool)
        .unwrap_err()
        .starts_with("invalid 'tool':"));

    let empty_tool = Map::from_iter([
        ("tool".into(), json!("  ")),
        ("connection_id".into(), json!(7)),
    ]);
    assert_eq!(
        parse_execute(empty_tool).unwrap_err(),
        "'tool' must not be empty"
    );

    let invalid_connection = Map::from_iter([
        ("tool".into(), json!("GMAIL_SEND_EMAIL")),
        ("connection_id".into(), json!(7)),
    ]);
    assert!(parse_execute(invalid_connection)
        .unwrap_err()
        .starts_with("invalid 'connection_id':"));
}
