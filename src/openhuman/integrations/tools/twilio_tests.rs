use super::*;

#[test]
fn tool_metadata() {
    let client = Arc::new(IntegrationClient::new("http://test".into(), "tok".into()));
    let tool = TwilioCallTool::new(client);
    assert_eq!(tool.name(), "twilio_call");
    assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    assert_eq!(tool.scope(), ToolScope::CliRpcOnly);
    assert!(tool.description().contains("phone call"));
}

#[test]
fn schema_has_required_to() {
    let client = Arc::new(IntegrationClient::new("http://test".into(), "tok".into()));
    let tool = TwilioCallTool::new(client);
    let schema = tool.parameters_schema();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["to"].is_object());
    let required = schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v == "to"));
}

#[tokio::test]
async fn execute_rejects_missing_to() {
    let client = Arc::new(IntegrationClient::new("http://test".into(), "tok".into()));
    let tool = TwilioCallTool::new(client);
    let result = tool.execute(json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn execute_rejects_empty_to() {
    let client = Arc::new(IntegrationClient::new("http://test".into(), "tok".into()));
    let tool = TwilioCallTool::new(client);
    let result = tool.execute(json!({"to": ""})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("empty"));
}

#[tokio::test]
async fn execute_rejects_no_content() {
    let client = Arc::new(IntegrationClient::new("http://test".into(), "tok".into()));
    let tool = TwilioCallTool::new(client);
    let result = tool.execute(json!({"to": "+14155551234"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("message"));
}

#[test]
fn twilio_response_deserializes() {
    let json = r#"{"callSid":"CA123","status":"queued","costUsd":0.03}"#;
    let resp: TwilioCallResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.call_sid, "CA123");
    assert_eq!(resp.status, "queued");
    assert!((resp.cost_usd - 0.03).abs() < f64::EPSILON);
}
